//! Main per-frame update tick (`perform_hourglass`).

use super::*;
use crate::abilities::{self, BeginResult as AbilityBeginResult};
use crate::bow_shot::{self, BeginShotResult};
use crate::element::{Command, Entity, EntityId};
use crate::game_operation::GameCode;
use crate::geo2d::{self, Point2D};
use crate::messenger::{Message, MessageType, SimpleMessage};
use crate::profiles::MissionType;

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
    /// Wraps [`EngineInner::perform_hourglass_inner`] with simulation-RNG
    /// install/uninstall and the deferred sound-queue drain so all
    /// gameplay-affecting randomness is pulled from the owned
    /// [`EngineInner::rng`] (deterministic across clients) and all audio is
    /// flushed *after* the sim is done (letting rollback replay the tick
    /// without duplicating playback).
    pub fn perform_hourglass(
        &mut self,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
        dev: &mut DevState,
    ) -> super::SideEffects {
        let _hourglass_timer = HourglassTimer::start();
        // Move the real RNG into the thread-local so sim helpers can pull
        // from it without threading `&mut fastrand::Rng` through every
        // signature. A placeholder lives on the struct while the tick runs;
        // it's replaced with the advanced state after uninstall.
        //
        // A panic inside the tick will leak the RNG in the thread-local
        // for this thread — acceptable because a sim-tick panic is already
        // fatal to the running game.
        #[allow(clippy::disallowed_methods)]
        let placeholder = fastrand::Rng::with_seed(0);
        crate::sim_rng::install(std::mem::replace(&mut self.rng, placeholder));

        let code = self.perform_hourglass_inner(display, assets, dev);

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
        display.tick_macro_shift_phases(&self.pc_ids, &self.macro_store);
        // Advance per-PC QA titbit fizzle-blink phase.  Host renderer
        // reads visibility via `macro_titbit_blink_hidden`.
        display.tick_macro_blink_phases(&self.pc_ids);
        // Advance destination-marker animation and retire finished
        // marks.  Used to run during rendering, which broke rollback
        // determinism — the render path is now read-only.
        {
            let view_pos = self.cutscene_camera.view_position;
            let zoom = self.cutscene_camera.zoom_factor;
            let screen = Self::director_camera_view_size();
            let screen_w = screen.x as i32;
            let screen_h = screen.y as i32;
            let frame_counter = self.frame_counter;
            self.ground_mark
                .tick(view_pos, zoom, screen_w, screen_h, frame_counter);
        }
        // Sound-source delay state machine — fully sim-side now: engine
        // ticks the timer down, fires a `PlayDelayedSource` side-effect
        // when it hits zero, and re-rolls the next delay using
        // `sim_rng`. The host just consumes the command to kick off
        // audio playback. Previously the timer reset lived host-side
        // (driven by audio-backend completion + a host RNG), which
        // broke rollback determinism.
        let num_sources = self.sound_sim.sources.num_sources();
        for i in 0..num_sources {
            let Some(src) = self.sound_sim.sources.get_mut(i) else {
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
                    let step = crate::sim_rng::u32(0..src.delay_stepping as u32) as u16;
                    let range = src.max_delay - src.min_delay;
                    src.timer = (step as u32 * range as u32 / src.delay_stepping as u32) as u16
                        + src.min_delay;
                } else {
                    src.timer = src.min_delay;
                }
                self.pending_side_effects
                    .sounds
                    .push(super::SoundCommand::PlayDelayedSource(i));
            }
        }

        let skip_render = self.tick_camera_display_state();

        // Reset per-frame scroll dedupe after the camera display tick.
        // Host-local viewport scroll is host-side and never enters engine
        // state, so peer-2's held scroll doesn't gate the host's, and vice
        // versa.
        self.cutscene_camera.display.frame_scrolled = [false; 4];
        display.frame_scrolled = [false; 4];

        self.rng = crate::sim_rng::uninstall();

        let mut fx = std::mem::take(&mut self.pending_side_effects);
        fx.code = code;
        fx.skip_render = skip_render != 0;
        fx
    }

    /// Whether any PC is currently guarded.
    pub fn is_pc_guarded(&self) -> bool {
        for &pc_id in &self.pc_ids {
            if let Some(Entity::Pc(pc)) = self.get_entity(pc_id)
                && pc.pc.guard.is_some()
            {
                return true;
            }
        }
        false
    }

    /// Decrement `wait_time` for every actor whose current in-progress
    /// sequence element is `Command::WaitTimer`.  When the counter
    /// reaches 0, fire `element_terminated` on that element so the
    /// next hourglass pass advances past it.
    fn tick_actor_wait_timers(&mut self) {
        if self.freeze_all {
            return;
        }
        // Two-pass to avoid overlapping borrows of `self.entities`
        // and `self.sequence_manager`.
        struct Pending {
            owner: EntityId,
            seq_id: crate::sequence::SequenceId,
            elem_idx: usize,
            terminate: bool,
        }
        let mut pending: Vec<Pending> = Vec::new();
        for (idx, slot) in self.entities.iter().enumerate() {
            let Some(entity) = slot else { continue };
            if entity.actor_data().is_none() {
                continue;
            }
            let owner = EntityId(idx as u32);
            let Some((seq_id, elem_idx)) = self.sequence_manager.current_element_for_actor(owner)
            else {
                continue;
            };
            let Some(elem) = self.sequence_manager.get_element(seq_id, elem_idx) else {
                continue;
            };
            if elem.command != crate::element::Command::WaitTimer {
                continue;
            }
            pending.push(Pending {
                owner,
                seq_id,
                elem_idx,
                terminate: false,
            });
        }
        for p in &mut pending {
            if let Some(Some(entity)) = self.entities.get_mut(p.owner.0 as usize)
                && let Some(actor) = entity.actor_data_mut()
            {
                if actor.wait_time == 0 {
                    p.terminate = true;
                } else {
                    actor.wait_time -= 1;
                }
            }
        }
        for p in pending {
            if p.terminate {
                self.sequence_manager
                    .element_terminated(p.seq_id, p.elem_idx);
            }
        }
    }

    fn perform_hourglass_inner(
        &mut self,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
        dev: &mut DevState,
    ) -> GameCode {
        // ── FadeToBlack stop-the-world freeze ────────────────────────
        // The script-driven fade-to-black calls
        // `Flip()` `2*speed` times in a tight loop with no engine update
        // between iterations, so the game is frozen for the duration of
        // the fade. While `frozen_until_frame > frame_counter`, skip all
        // game logic and just bump the counter — the host renderer is
        // still called by the outer game loop and drains the fade ramp
        // per render frame. Once the deadline hits, clear the freeze
        // and fall through to a normal tick.
        if let Some(deadline) = self.frozen_until_frame {
            if self.frame_counter < deadline {
                self.frame_counter += 1;
                return GameCode::LevelInProgress;
            }
            self.frozen_until_frame = None;
        }

        // Drain deferred console-cheat / death reinforcement spawns and
        // scroll-reveal amulet spawns. Both used to live in
        // `Game::run_engine_tick` because they needed `&mut LevelAssets`
        // to load sprites; the two sprite families are now preloaded at
        // mission start (`preload_campaign_peasant_sprites`,
        // `preload_scroll_amulet_sprite`) so the spawn paths read the
        // scriptor cache via `&LevelAssets` and the whole flow lives
        // inside `perform_hourglass` — keeping the "sim mutation only
        // during perform_hourglass" invariant intact.
        self.drain_pending_reinforcements(assets);
        self.drain_pending_scroll_amulets(assets);
        self.drain_pending_hero_speeches(assets);
        self.drain_pending_hades_kills(assets);
        self.drain_pending_concussion_side_effects(assets);

        // Drain matured exclamations into `finished_exclamations` so the
        // AI MYTALK handler (later in this tick) sees them. Used to be
        // populated host-side by audio-backend playback completion,
        // which made rollback non-deterministic — now scheduled at
        // emit time using the host-supplied `exclamation_durations`
        // table.
        let cur_frame = self.frame_counter;
        let mut still_playing = Vec::new();
        let mut finished = Vec::new();
        for p in self.sound_sim.playing_exclamations.drain(..) {
            if p.finish_frame <= cur_frame {
                finished.push((p.actor_id, p.exclamation_id));
            } else {
                still_playing.push(p);
            }
        }
        self.sound_sim.playing_exclamations = still_playing;
        self.sound_sim.finished_exclamations = finished;

        // Drain matured sound-source finishes.  Replaces the
        // `stop_sound_source` logic the Rust host used to run on
        // SDL_mixer playback-completion events: for each scheduled
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
        for p in self.sound_sim.playing_sources.drain(..) {
            if p.finish_frame > cur_frame {
                still_playing_sources.push(p);
                continue;
            }
            let Some(src) = self.sound_sim.sources.get(p.source_index as usize) else {
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
        self.sound_sim.playing_sources = still_playing_sources;
        for idx in source_deactivations {
            if let Some(src) = self.sound_sim.sources.get_mut(idx) {
                src.active = false;
            }
        }
        for idx in source_deletions {
            self.sound_sim.sources.delete(idx);
        }

        // PC-guarded state drives start/quit mission widget enable and
        // guard-portrait blinking.  The
        // widget-enable side is applied from `Game::run_engine_tick`
        // before `perform_hourglass` runs so both consumers see the
        // same value for this tick.  The guard-portrait blink is
        // rendered live by `ui_panel.rs` directly from
        // `mission.mission_won` + `PcData::guard`, so there's nothing
        // to do here for (b).
        let pc_guarded = self.is_pc_guarded();

        // ── Projectile cheat rain ────────────────────────────────
        // The original `ProjectileRain` cheat was wired up but never
        // implemented in the shipped build.  Preserve the drain so the
        // dev flag resets, but don't invent gameplay that never existed.
        if dev.projectile_cheat_rain >= 0 {
            dev.projectile_cheat_rain = -1;
        }

        // ── Anti-chorus timer ────────────────────────────────────
        if self.chorus_timer > 0 {
            self.chorus_timer -= 1;
        }

        // ── First-time mission-won message ───────────────────────
        // Fire the mission-state banner ("leave mission now" / quit
        // mission popup) and disable the quit-mission widget once the
        // player has reached a guarded exit AND no PC is currently
        // being guarded (guarded PCs can't lead everyone out yet).
        // We signal both via `SideEffects.pending_mission_state_notice`;
        // the host flips the widget-enable flag and shows the popup.
        if self.mission.mission_won_first_time && !pc_guarded {
            self.mission.mission_won_first_time = false;
            self.pending_side_effects.pending_mission_state_notice = true;
        }

        // ── Check quit conditions ────────────────────────────────
        // Each of the three quit branches displays the full minimap.
        if self.mission.quit_won {
            display.minimap.display_map(false, true);
            self.finalize_mission_script(false);
            return GameCode::LevelSucceeded;
        }
        if self.mission.quit_lost {
            display.minimap.display_map(false, true);
            self.quit_mission();
            return GameCode::LevelFailed;
        }
        if self.mission.quit_interrupted {
            display.minimap.display_map(false, true);
            self.finalize_mission_script(true);
            return GameCode::LevelInterrupted;
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
        if self.frame_counter.is_multiple_of(FRAMES_PER_SECOND) {
            let game_seconds = self.frame_counter / FRAMES_PER_SECOND;

            // Refresh entity active state so IsAnimationActive reads live data.
            self.refresh_game_host_entity_state();

            if let Some(ref mut script) = self.mission_script {
                if let Some(game_host) = script.game_host_mut() {
                    game_host.frame_counter = self.frame_counter;
                }
                script.swap_engine_state(
                    &mut self.entities,
                    &mut self.ai_global,
                    &mut self.fast_grid,
                    &mut self.campaign,
                    &mut self.mission_stat,
                );
                if let Err(e) = script.hourglass(game_seconds) {
                    tracing::warn!("Script Hourglass error: {e}");
                }
                script.swap_engine_state(
                    &mut self.entities,
                    &mut self.ai_global,
                    &mut self.fast_grid,
                    &mut self.campaign,
                    &mut self.mission_stat,
                );
            }

            // Apply changes the script made back to the engine.
            self.sync_game_host_post_script(assets);

            // Check victory/defeat conditions every 3 game-seconds
            // (or immediately if force_check was set by a native call).
            if game_seconds.is_multiple_of(VICTORY_CHECK_INTERVAL) || self.force_check {
                self.force_check = false;

                // Take the script out to avoid borrow conflicts with `self`.
                if let Some(mut script) = self.mission_script.take() {
                    if let Some(game_host) = script.game_host_mut() {
                        game_host.frame_counter = self.frame_counter;
                    }
                    script.swap_engine_state(
                        &mut self.entities,
                        &mut self.ai_global,
                        &mut self.fast_grid,
                        &mut self.campaign,
                        &mut self.mission_stat,
                    );
                    let victory_result = script.check_victory_condition(game_seconds);
                    script.swap_engine_state(
                        &mut self.entities,
                        &mut self.ai_global,
                        &mut self.fast_grid,
                        &mut self.campaign,
                        &mut self.mission_stat,
                    );

                    // Put the script back before syncing side-effects.
                    self.mission_script = Some(script);
                    self.sync_game_host_post_script(assets);

                    match victory_result {
                        Ok(1) => {
                            // Mission won!
                            if !self.mission.mission_won {
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
                            return GameCode::LevelFailed;
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
        self.frame_counter += 1;

        // ── Skip logic if engine is locked (zoom, sequence, etc) ─
        if display.background_transform.zoom_to_up
            || display.background_transform.zoom_to_down
            || self.lock_engine
        {
            return GameCode::LevelInProgress;
        }

        // ── Default lose condition check ─────────────────────────
        // Guarded by `ignore_default_loose`.
        // Missions that keep-an-NPC-alive (e.g. "protect the cart")
        // set this flag to true so the default "all PCs dead/guarded /
        // dead-PC / civilian-killed" loss checks are skipped; the
        // script's `CheckVictoryCondition` is the authority instead.
        let ignore_default_loose = crate::engine::GlobalOptions::global()
            .as_ref()
            .map(|o| o.ignore_default_loose)
            .unwrap_or(false);
        if !ignore_default_loose {
            // Check if any playable PC is alive and not guarded.
            // If no PCs are playable (all dead/unconscious/guarded), the mission is lost.
            if !self.pc_ids.is_empty() {
                let any_playable_and_free = self.pc_ids.iter().any(|&pc_id| {
                    if let Some(Some(Entity::Pc(pc))) = self.entities.get(pc_id.0 as usize) {
                        let alive = pc.pc.life_points > 0 && !pc.human.unconscious;
                        let guarded = pc.pc.guard.is_some();
                        alive && !guarded
                    } else {
                        false
                    }
                });
                if !any_playable_and_free {
                    tracing::info!("All PCs dead or guarded — mission lost");
                    self.quit_mission();
                    return GameCode::LevelFailed;
                }
            }

            // Check if a dead PC was flagged for mission failure
            if let Some(dead_id) = self.dead_pc.take() {
                if let Some(entity) = self.get_entity(dead_id) {
                    let pos = entity.element_data().position_map();
                    self.center_on_point(0, geo2d::pt(pos.x, pos.y));
                }
                self.quit_mission();
                return GameCode::LevelFailed;
            }

            // Check if any civilian was killed (not by accident) → mission failure
            let mut killed_civilian = None;
            for &npc_id in &self.npc_ids {
                if let Some(Some(entity)) = self.entities.get(npc_id.0 as usize)
                    && entity.is_civilian()
                    && entity.is_dead()
                {
                    // Check killed_by_accident via the civilian's human data
                    let accident = match entity {
                        Entity::Civilian(c) => c.human.killed_by_accident,
                        _ => false,
                    };
                    if !accident {
                        killed_civilian = Some(npc_id);
                        break;
                    }
                }
            }
            if let Some(civ_id) = killed_civilian {
                if let Some(entity) = self.get_entity(civ_id) {
                    let pos = entity.element_data().position_map();
                    self.center_on_point(0, geo2d::pt(pos.x, pos.y));
                }
                self.quit_mission();
                return GameCode::LevelFailed;
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
        let pc_ids_for_reinf: Vec<EntityId> = self.pc_ids.clone();
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
                self.pending_reinforcements.push(Some(pc_id));
            }
        }

        // ── Process messenger (engine-state messages) ────────────
        // Handle pending messages that mutate engine state. Other
        // messages (UI/mission flow) are left in the queue for their
        // respective consumers (UI layer, tests, etc.) to observe.
        // We only consume the ones that actually affect engine state.
        {
            let messages = self.messenger.drain();
            for msg in messages {
                match msg.msg_type {
                    MessageType::Simple(SimpleMessage::LockAlt) => {
                        self.seats[0].is_lock_alt = true;
                    }
                    MessageType::Simple(SimpleMessage::UnlockAlt) => {
                        self.seats[0].is_lock_alt = false;
                    }
                    // Macro recording state machine.  The PC id is
                    // passed via the message: a present id targets one
                    // specific PC; an absent id arms every currently-
                    // selected PC.
                    MessageType::Pc(crate::messenger::PcMessage::StartRecordingMacro, pc) => {
                        let slot = self.qa_recording_slot;
                        let targets: Vec<crate::element::EntityId> = match pc {
                            Some(id) => vec![id],
                            None => self.seats[0].selection.clone(),
                        };
                        for pc_id in &targets {
                            self.macro_store.get_or_insert(*pc_id).begin_recording(slot);
                        }
                        self.qa_recording_for = targets;
                        // Snapshot the currently-armed action so the
                        // MSG_STOP_RECORDING_MACRO post-process can
                        // restore it.
                        self.action_before_recording_macro = self.get_selected_action();
                    }
                    MessageType::Pc(crate::messenger::PcMessage::StopRecordingMacro, _) => {
                        // Suppress the post-process restore unless
                        // something was actually recording.
                        let was_recording = !self.qa_recording_for.is_empty();
                        for pc_id in self.qa_recording_for.clone() {
                            if let Some(state) = self.macro_store.get_mut(pc_id) {
                                state.stop_recording();
                            }
                        }
                        self.qa_recording_for.clear();

                        // Post-process: re-select the action that was
                        // armed before recording started.  Apply the
                        // saved action to each selected PC directly —
                        // we do not route MSG_SELECT_ACTION through
                        // the messenger drain.
                        if was_recording {
                            let restore = self.action_before_recording_macro;
                            self.action_before_recording_macro = crate::profiles::Action::NoAction;
                            for id in self.seats[0].selection.clone() {
                                if let Some(entity) = self.get_entity_mut(id)
                                    && let Some(pc) = entity.pc_data_mut()
                                {
                                    pc.current_action = restore;
                                }
                            }
                            // Emit the message for script /
                            // edge-subscriber observation.
                            self.messenger
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
                        if !self.qa_recording_for.is_empty() {
                            let slot = self.qa_recording_slot;
                            let selected: Vec<crate::element::EntityId> =
                                self.seats[0].selection.clone();
                            // End on PCs that left the selection.
                            let current = self.qa_recording_for.clone();
                            for pc_id in &current {
                                if !selected.contains(pc_id)
                                    && let Some(state) = self.macro_store.get_mut(*pc_id)
                                {
                                    state.stop_recording();
                                }
                            }
                            // Start on PCs newly selected.
                            for pc_id in &selected {
                                if !current.contains(pc_id) {
                                    self.macro_store.get_or_insert(*pc_id).begin_recording(slot);
                                }
                            }
                            self.qa_recording_for = selected;
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
                        self.pending_side_effects
                            .sounds
                            .push(super::SoundCommand::Jingle(
                                crate::sound::Jingle::NewPeasantCalled,
                            ));
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
                        self.pending_side_effects.ui_has_focus = true;
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
                                    let ids = self.pc_ids.clone();
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
                        self.pending_side_effects.pending_reset_input = true;
                        self.pending_side_effects.reset_input = true;
                        // Clear the alt-lock latch along with the
                        // modifier cache; without this, an alt-lock
                        // toggled before a console-hide / task-switch
                        // / save-load / unlock-user would persist
                        // past the reset.
                        self.seats[0].is_lock_alt = false;
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
                        self.user_locked = true;
                    }
                    MessageType::Simple(crate::messenger::SimpleMessage::UnlockUser) => {
                        self.user_locked = false;
                        self.pending_side_effects.pending_reset_input = true;
                    }
                    // After hiding the console or switching task,
                    // emit `MSG_RESET_INPUT` so the held-key edges
                    // and modifier latches don't bleed across the
                    // task boundary.
                    MessageType::Simple(crate::messenger::SimpleMessage::HideConsole)
                    | MessageType::Simple(crate::messenger::SimpleMessage::SwitchTask) => {
                        self.pending_side_effects.pending_reset_input = true;
                        self.pending_side_effects.reset_input = true;
                        // Same `is_lock_alt` clear as the explicit
                        // `ResetInput` arm above.
                        self.seats[0].is_lock_alt = false;
                    }
                    // `SelectActionSimple` and `DisableAction` both
                    // clear the aim-trajectory preview so a dropped /
                    // replaced action doesn't leave a stale trajectory
                    // overlay on screen.  `valid_trajectory` lives on
                    // `host` in the Rust split, so raise the
                    // side-effect flag.
                    MessageType::Pc(crate::messenger::PcMessage::SelectActionSimple, _)
                    | MessageType::Pc(crate::messenger::PcMessage::DisableAction, _) => {
                        self.pending_side_effects.invalidate_trajectory_preview = true;
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
                                let pc_ids = self.pc_ids.clone();
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
                    _ => self.messenger.send(msg),
                }
            }
        }

        // ── Sequence manager cleanup ─────────────────────────────
        // Run every 256 frames (or every frame in debug).
        if self.frame_counter.is_multiple_of(256) {
            self.sequence_manager.friday_evening_cleanup();
        }

        // ── Process pending AI orders ─────────────────────────────
        //
        // AI Move intents collected by `launch_pending_orders_for_npc`
        // route through `launch_ai_move`, which just enqueues into
        // `pending_move_requests` (dedup-per-actor).  The drain below
        // promotes one Move sequence element per unique actor this
        // tick — absorbing redundant re-fires that would otherwise
        // launch a fresh Move each frame and `InterruptCurrent` the
        // in-flight one.  We skip the per-frame rate-limit the
        // original used — Rust's A* is fast enough to resolve every
        // pending actor in a single tick.
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
        self.dispatch_pending_waypoint_scripts(assets);

        // ── Process cross-NPC actions (phalanx coordination) ────
        self.process_pending_cross_npc_actions(assets);

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
        // sprite animation completes (detected in tick_entity_animations).
        self.process_animation_orders();

        // Pathfinding is synchronous now — Move sequence elements
        // call `find_path` directly when their `InstructOwner` action
        // dispatches (see the Move dispatch in this file).  The
        // legacy async `ProcessPathRequests` drain had no remaining
        // producers post-refactor and was deleted.

        // Snapshot pre-hourglass swordfight state so we can detect a
        // swordfight→non-swordfight transition across this tick and
        // raise the ignore-mouse-event bracket on the falling edge.
        // The per-element / sequence-manager hourglass passes below may
        // flip the selected PC out of `Swordfighting`; when that
        // happens mid-drag the in-flight drag must be suppressed so it
        // doesn't bleed into the next click-release action.
        let was_swordfighting = self.is_selected_pc_swordfighting();

        // ── Element hourglass (per-element update) ───────────────
        let mut to_remove = Vec::new();
        for (idx, slot) in self.entities.iter_mut().enumerate() {
            if let Some(entity) = slot
                && !entity.hourglass()
            {
                to_remove.push(EntityId(idx as u32));
            }
        }
        for id in to_remove {
            self.remove_entity(id);
        }

        // ── Bonus auto-pickup ─────────────────────────────────────
        // Check if any PC walked within pickup radius of a bonus item.
        self.tick_bonus_auto_pickup(assets);

        // ── PC selection outline fade ────────────────────────────
        // The hulk state-machine block runs during the per-element
        // refresh pass.
        self.refresh_pc_selection_hulk();

        // Tick the cheat-teleport hulk-rebuild fade counter on every
        // PC.  Decrementing here (rather than from the per-PC render
        // path) lets rollback / replay see bit-identical state (the
        // counter is serde'd `PcData`).
        self.tick_pc_teleport_fades();

        // ── Failed-path retry ────────────────────────────────────
        // Move / Seek elements whose pathfind failed on a previous
        // tick stay in `InProgress` with empty orders for up to 100
        // frames while the engine retries.  Successful retries
        // populate orders; timeouts mark the element `Impossible` and
        // fire `HERO_UNABLE_TO_DO_SOMETHING` for PCs.  Runs before the
        // hourglass dispatch so same-tick failures & retries both age
        // correctly.
        self.process_failed_path_timeouts(assets);

        // ── Per-tick RefreshSeek ─────────────────────────────────
        // For every actor with an in-flight Command::Seek whose
        // target has moved >10 units since the last seek launch (and
        // `seek_refresh_wait` has elapsed), interrupt the current
        // movement element and launch a fresh seek sequence.  Runs
        // before the hourglass so the relaunched seek dispatches in
        // the same tick.  See [`engine/refresh_seek.rs`] for the
        // covered and still-outstanding branches.
        self.tick_refresh_seeks(assets);

        // ── Actor WAIT_TIMER countdown ───────────────────────────
        // For every actor whose current sequence element is
        // `Command::WaitTimer`, decrement `wait_time`; when it
        // reaches 0 the element transitions to terminated.
        self.tick_actor_wait_timers();

        // ── Sequence manager dispatch ────────────────────────────
        // Process pending sequence elements and dispatch actions.
        // We collect actions and process them here in two passes.
        let actions = self.sequence_manager.hourglass();

        // First pass: extract Move command data (to avoid borrow conflicts).
        // (owner, seq_id, elem_idx, destination, layer, action_animation)
        let mut move_instructions: Vec<(
            EntityId,
            crate::sequence::SequenceId,
            usize,
            Point2D,
            u16,
            crate::order::OrderType,
        )> = Vec::new();
        // Beggar-command rejections collected during the Move-gather
        // pass — applied after the loop to avoid `&sequence_manager`
        // vs `&mut sequence_manager` borrow conflicts.
        let mut beggar_rejects_pass1: Vec<(crate::sequence::SequenceId, usize)> = Vec::new();
        // Per-actor instruct arbitration.  Runs once per owner so the
        // set of "current" elements observed is consistent across pass 1
        // and pass 2 dispatchers.  Element handles that fail arbitration
        // (Abandon / Postpone) are collected here so we skip them in
        // both passes below.
        let mut abandoned_or_postponed: std::collections::HashSet<(
            crate::sequence::SequenceId,
            usize,
        )> = std::collections::HashSet::new();
        for action in &actions {
            if let crate::sequence::SequenceAction::InstructOwner {
                owner,
                sequence_id,
                element_index,
            } = action
            {
                if !self.arbitrate_instruct(*sequence_id, *element_index) {
                    abandoned_or_postponed.insert((*sequence_id, *element_index));
                    continue;
                }

                let needs_transition = self
                    .sequence_manager
                    .get_element(*sequence_id, *element_index)
                    .is_some_and(|elem| {
                        matches!(
                            elem.state,
                            crate::sequence::SequenceState::Todo
                                | crate::sequence::SequenceState::Postponed
                        ) && elem.posture_after_transition == crate::element::Posture::Undefined
                    });
                if needs_transition
                    && !self.generate_transition(*owner, *sequence_id, *element_index)
                {
                    self.sequence_manager
                        .element_impossible(*sequence_id, *element_index);
                    abandoned_or_postponed.insert((*sequence_id, *element_index));
                }
            }
        }

        for action in &actions {
            if let crate::sequence::SequenceAction::InstructOwner {
                owner,
                sequence_id,
                element_index,
            } = action
                && !abandoned_or_postponed.contains(&(*sequence_id, *element_index))
                && let Some(elem) = self
                    .sequence_manager
                    .get_element(*sequence_id, *element_index)
                // `Command::Seek` shares the pathfinder dispatch with
                // `Command::Move`.  Without this fall-through, Seek
                // elements (used by the seek-before-take object
                // pickup sequence) would be silently terminated by
                // the default arm in the second pass instead of
                // walking the PC up to their target.
                && matches!(
                    elem.command,
                    crate::element::Command::Move | crate::element::Command::Seek
                )
                && let crate::sequence::SequenceElementData::Movement {
                    destination,
                    element,
                    layer,
                    sector: _,
                    action,
                    flags,
                    tolerance,
                    ..
                } = &elem.data
            {
                let stored_destination = *destination;
                let target_element = *element;
                let instr_layer = *layer;
                let instr_action = *action;
                let instr_flags = *flags;
                let instr_tolerance = *tolerance;
                let is_seek = elem.command == crate::element::Command::Seek;
                // Beggars reject Move (and anything except
                // RECEIVE_PURSE / BEGGAR_SHOW_FACE / WAIT).  Mark
                // impossible and skip the pathfind.
                if self.beggar_rejects_command(*owner, crate::element::Command::Move) {
                    beggar_rejects_pass1.push((*sequence_id, *element_index));
                    continue;
                }
                // An anonymous-archer PC (archery-contest disguise)
                // cannot move; play HERO_UNABLE_TO_DO_SOMETHING and
                // mark the Move element Impossible so any chained
                // sequence sees the failure rather than falling
                // through to the pathfinder.  The check covers both
                // Move and Seek (Seek falls through to Move).
                let is_anonymous_archer_pc = self.get_entity(*owner).is_some_and(|e| {
                    e.is_pc()
                        && e.element_data().posture
                            == crate::element_kinds::Posture::AnonymousArcher
                });
                if is_anonymous_archer_pc {
                    self.hero_speaking(
                        assets,
                        *owner,
                        crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                    );
                    self.sequence_manager
                        .element_impossible(*sequence_id, *element_index);
                    continue;
                }
                // Seek resolves the destination from the target
                // entity's current position at dispatch time.
                // `InstructOwner` fires once per element launch, so
                // this is a one-shot snapshot — no per-tick re-read.
                // Plain Move uses the stored `destination` point.
                let dest_pt = if is_seek {
                    let post_seek = self
                        .sequence_manager
                        .get_element_mut(*sequence_id, *element_index)
                        .and_then(|elem| match &mut elem.data {
                            crate::sequence::SequenceElementData::Movement {
                                post_seek_sequence,
                                ..
                            } => post_seek_sequence.take(),
                            _ => None,
                        });
                    if let Some(post_seek) = post_seek
                        && let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                        && let Some(actor) = entity.actor_data_mut()
                    {
                        actor.post_seek_sequence = Some(post_seek);
                    }

                    match target_element {
                        Some(target) => {
                            if target == *owner {
                                self.sequence_manager
                                    .element_terminated(*sequence_id, *element_index);
                                self.start_post_seek_sequence(*owner, None);
                                continue;
                            }
                            if self.try_handle_same_sector_actor_seek_wait(
                                *owner,
                                *sequence_id,
                                *element_index,
                                target,
                                instr_flags,
                            ) {
                                continue;
                            }
                            // Entity-target SEEK floors the seek
                            // distance at 4.0 before stamping it on
                            // the actor and feeding it to RefreshSeek.
                            // Without the floor, NPCs chasing a target
                            // with a small element-tolerance pause
                            // every refresh because the pathfinder
                            // thinks they've already arrived.
                            let floored_seek_distance = instr_tolerance.max(4.0);
                            if self.try_dispatch_cross_sector_entity_seek(
                                assets,
                                *owner,
                                *sequence_id,
                                *element_index,
                                target,
                                instr_action,
                                instr_flags,
                                floored_seek_distance,
                            ) {
                                continue;
                            }
                            let Some(resolved) = self.resolve_entity_seek(
                                *owner,
                                target,
                                instr_flags,
                                floored_seek_distance,
                            ) else {
                                beggar_rejects_pass1.push((*sequence_id, *element_index));
                                continue;
                            };
                            if let Some(elem_mut) = self
                                .sequence_manager
                                .get_element_mut(*sequence_id, *element_index)
                                && let crate::sequence::SequenceElementData::Movement {
                                    destination,
                                    tolerance,
                                    speed_factor,
                                    ..
                                } = &mut elem_mut.data
                            {
                                *destination = crate::element::Point2D {
                                    x: resolved.destination.x,
                                    y: resolved.destination.y,
                                };
                                *tolerance = resolved.tolerance;
                                *speed_factor = resolved.speed_factor;
                            }
                            // Arm the actor's seek-refresh wait;
                            // seek-distance / seek-to-point live on
                            // the movement element.
                            if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                                && let Some(actor) = entity.actor_data_mut()
                            {
                                actor.seek_refresh_wait = 25;
                            }
                            if resolved.stop_npc {
                                self.send_seek_stop_to_npc(target);
                            }
                            resolved.destination
                        }
                        None => {
                            // Point-target SEEK: the layer / sector /
                            // tolerance live on the movement element;
                            // keep the actor refresh stamp coherent.
                            if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                                && let Some(actor) = entity.actor_data_mut()
                            {
                                actor.seek_target = None;
                                actor.last_seek_target_position = stored_destination;
                                actor.seek_refresh_wait = 25;
                            }
                            stored_destination.to_geo_point()
                        }
                    }
                } else {
                    stored_destination.to_geo_point()
                };
                // Move (or Seek that fell through to Move) inside a
                // building sector skips the pathfinder entirely:
                // position is snapped to the destination and the
                // element terminates.  The exception is
                // `(SEEK && IsLastElementOfSequence)`, which either
                // launches the post-seek sequence or emits a
                // REFRESHING_SEEK order — that branch is already
                // partially covered by the SEEK_IN_BUILDINGS handling
                // earlier in this loop and stays on the pathfinder
                // path here so the existing post-seek flow remains in
                // charge.
                let owner_sector = self
                    .get_entity(*owner)
                    .and_then(|e| e.element_data().sector());
                let owner_in_building = self.sector_is_building(owner_sector);
                let is_last_of_seq = self
                    .sequence_manager
                    .get_sequence(*sequence_id)
                    .map(|s| *element_index + 1 >= s.elements.len())
                    .unwrap_or(false);
                if owner_in_building && (!is_seek || !is_last_of_seq) {
                    if let Some(entity) = self.get_entity_mut(*owner) {
                        entity.position_iface_mut().set_position_map(dest_pt);
                        let dest_pt_elem = crate::element::Point2D {
                            x: dest_pt.x,
                            y: dest_pt.y,
                        };
                        entity.element_data_mut().set_position_map(dest_pt_elem);
                        entity.element_data_mut().update_grid_cell();
                    }
                    self.sequence_manager
                        .element_terminated(*sequence_id, *element_index);
                    continue;
                }
                move_instructions.push((
                    *owner,
                    *sequence_id,
                    *element_index,
                    dest_pt,
                    instr_layer,
                    instr_action,
                ));
            }
        }
        for (seq_id, elem_idx) in beggar_rejects_pass1 {
            self.sequence_manager.element_impossible(seq_id, elem_idx);
        }

        // Process Move instructions: pathfind and set up entity movement.
        for (owner, seq_id, elem_idx, dest, _layer, move_action) in move_instructions {
            // NOTE: posture transitions (leave-disguise, stand-up, …)
            // are handled at launch time by `generate_transition` via
            // the engine-side `launch_element_for_owner` / stamped
            // single-order-sequence wrappers.  The older
            // `auto_leave_disguise_if_needed` dispatch hook that used
            // to fire here has been superseded.
            //
            // Sword-variant override and the pathfind + populate
            // pipeline both live inside `try_dispatch_move_path` so
            // the same code path is reused by the failed-path retry
            // pass.

            match self.try_dispatch_move_path(assets, owner, seq_id, elem_idx, dest, move_action) {
                crate::engine::movement::MovePathOutcome::Success => {}
                crate::engine::movement::MovePathOutcome::ActorGone => {
                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                }
                crate::engine::movement::MovePathOutcome::Failed => {
                    // Stamp the failed request with the current frame
                    // counter and push it onto `failed_path_requests`.
                    // The element stays `InProgress` with an empty
                    // order queue and sits there for up to 100 frames;
                    // no re-dispatch is attempted during the window.
                    // `process_failed_path_timeouts` then transitions
                    // the element to `Impossible` (and, for PCs,
                    // fires `HERO_UNABLE_TO_DO_SOMETHING`).
                    self.failed_path_requests
                        .push(crate::engine::movement::FailedPathRequest {
                            owner,
                            seq_id,
                            elem_idx,
                            first_fail_frame: self.frame_counter,
                        });
                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                }
            }
        }

        // Deferred script ProcessMessage calls — collected during the action
        // loop below and dispatched after iteration to avoid borrow conflicts.
        let mut deferred_process_messages: Vec<(i32, i32, i32, i32)> = Vec::new(); // (handle, msg, arg1, arg2)
        let mut deferred_engine_messages: Vec<(i32, i32, i32)> = Vec::new(); // (msg, arg1, arg2)
        // Deferred `IElementTargetScript::ActivatedBy*(pPC)` calls
        // collected from `Command::Activate*` sequence elements.
        // Entries are `(target_handle, pc_handle, method_name)`;
        // dispatched after the action loop via
        // `dispatch_target_activations`.
        let mut pending_target_activations: Vec<(i32, i32, &'static str)> = Vec::new();

        // Second pass: handle non-Move actions.
        //
        // Pop actions one at a time and drain any synchronous
        // immediate-dispatch follow-ups produced by cascades inside
        // each action (e.g. an `element_terminated` whose
        // `signal_ready` re-registers the next element which happens
        // to be Speak / Teleport / etc.).  Successors land at the
        // front of the action queue, so they fire before the next
        // non-immediate action in the batch rather than waiting for
        // the next `Hourglass()`.
        let mut actions: std::collections::VecDeque<crate::sequence::SequenceAction> =
            actions.into();
        while let Some(action) = actions.pop_front() {
            match action {
                crate::sequence::SequenceAction::InstructOwner {
                    owner,
                    sequence_id: seq_id,
                    element_index: elem_idx,
                } => {
                    // Skip elements rejected by the instruct
                    // arbitration (Abandon or Postpone).
                    if abandoned_or_postponed.contains(&(seq_id, elem_idx)) {
                        continue;
                    }
                    // Skip elements whose state moved to terminal /
                    // interrupted while another action in this batch
                    // arbitrated against them (e.g. a higher-priority
                    // element launched later in pass 1a cascaded an
                    // `InterruptCurrent` onto this one).  Without this,
                    // pass 2 would try to dispatch a non-live element
                    // and hit `set_element_state: Terminated from
                    // illegal state Interrupted`.
                    let cmd = match self.sequence_manager.get_element(seq_id, elem_idx) {
                        Some(e) => {
                            use crate::sequence::SequenceState;
                            if !matches!(e.state, SequenceState::Todo | SequenceState::Postponed) {
                                continue;
                            }
                            e.command
                        }
                        None => continue,
                    };
                    // Beggar-command filter: reject anything other
                    // than RECEIVE_PURSE / BEGGAR_SHOW_FACE / WAIT on
                    // beggar civilians.
                    if self.beggar_rejects_command(owner, cmd) {
                        self.sequence_manager.element_impossible(seq_id, elem_idx);
                        continue;
                    }
                    // Posture transitions (leave-disguise, stand-up, …)
                    // are handled before command dispatch: owned
                    // single-element launches do it in
                    // `launch_element_for_owner`, and prebuilt
                    // `launch_sequence` elements do it in the
                    // InstructOwner admission pass above.
                    //
                    // Re-borrow element for data access.
                    let elem = match self.sequence_manager.get_element(seq_id, elem_idx) {
                        Some(e) => e,
                        None => continue,
                    };
                    // Pre-flight re-validation, humans only — non-
                    // human owners (e.g. script-driven objects) skip
                    // the gate because the validity check only
                    // applies to humans.  Passes `check_position =
                    // true` to match the default at all call sites.
                    let owner_is_human = self
                        .get_entity(owner)
                        .map(|e| e.is_human())
                        .unwrap_or(false);
                    if owner_is_human
                        && !self.check_sequence_element_validity(assets, owner, elem, true)
                    {
                        self.sequence_manager.element_impossible(seq_id, elem_idx);
                        continue;
                    }
                    match cmd {
                        Command::Move | Command::Seek => {
                            // Already handled in the first pass above
                            // (Seek falls through to the Move
                            // dispatch — they share the same case).
                        }
                        Command::ShootBow | Command::ShootBowOnce => {
                            let shoot_once = cmd == Command::ShootBowOnce;
                            let antagonist = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            let target = match antagonist {
                                Some(t) => t,
                                None => {
                                    // No target — nothing we can do.
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                    continue;
                                }
                            };
                            // Check ammo before starting the shot
                            // (PCs only).  Zero bow ammo → impossible.
                            // Soldiers have unlimited ammo (no ammo
                            // counter).
                            let ammo_count = self.get_bow_ammo_count(owner);
                            if ammo_count == 0 {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            }

                            // Determine shoot mode via
                            // `can_shoot_with_bow_at` before
                            // beginning the shot.
                            let (bow_target, shoot_mode) =
                                self.can_shoot_with_bow_at(assets, owner, target);
                            if bow_target != super::input::BowTarget::Valid {
                                tracing::debug!(
                                    ?owner,
                                    ?target,
                                    ?bow_target,
                                    "ShootBow command rejected during dispatch"
                                );
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            }

                            match bow_shot::begin_bow_shot(
                                &mut self.entities,
                                &mut self.sequence_manager,
                                owner,
                                target,
                                seq_id,
                                elem_idx,
                                shoot_once,
                                ammo_count,
                                Some(shoot_mode),
                                &mut self.next_order_id,
                            ) {
                                BeginShotResult::Started => {
                                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                                }
                                BeginShotResult::Impossible => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::PassDoor => {
                            // Cross-layer door/lift transition.
                            //
                            // Builds a multi-step sub-order chain via
                            // `build_door_pass()` with correct
                            // animations per door type (building,
                            // ladder, wall, stairs, default).  The
                            // movement tick processes steps one at a
                            // time.
                            if let crate::sequence::SequenceElementData::Movement {
                                gate_id,
                                flags,
                                ..
                            } = &elem.data
                            {
                                let door_idx = match gate_id {
                                    Some(idx) => *idx,
                                    None => {
                                        self.sequence_manager.element_impossible(seq_id, elem_idx);
                                        continue;
                                    }
                                };

                                // Determine direction from the sequence element.
                                // `flags` has DOOR set; `direct` is inferred from
                                // which side the actor is on vs the door's sectors.
                                let direct = {
                                    // Snapshot door's sector_out to avoid
                                    // overlapping borrows.
                                    let door_sector_out = self
                                        .mission_script
                                        .as_mut()
                                        .and_then(|s| s.game_host_mut())
                                        .and_then(|h| h.doors.get(usize::from(door_idx)))
                                        .map(|d| d.sector_out);
                                    let actor_sector = self
                                        .get_entity(owner)
                                        .and_then(|e| e.element_data().sector());
                                    match (door_sector_out, actor_sector) {
                                        (Some(ds), Some(as_)) => u16::from(as_) == ds,
                                        _ => true,
                                    }
                                };

                                // ── Authorization check ──
                                // Verify the actor may use the door
                                // before building the step chain.
                                let auth_info = match self.get_entity(owner) {
                                    Some(e) => e.actor_auth_info(),
                                    None => {
                                        self.sequence_manager.element_impossible(seq_id, elem_idx);
                                        continue;
                                    }
                                };
                                let allow_leave_map =
                                    flags.contains(crate::sequence::MoveFlags::MAP);
                                // `building_has_capacity = true`
                                // always: building max-occupants is
                                // `0xFFFF` at construction and its
                                // proto load path is dead, so the
                                // capacity check always passes.  The
                                // parameter is kept on
                                // `is_actor_authorized` for the door
                                // struct's shape but has no live
                                // consumer.
                                let authorized = self
                                    .mission_script
                                    .as_mut()
                                    .and_then(|s| s.game_host_mut())
                                    .and_then(|h| h.doors.get(usize::from(door_idx)))
                                    .map(|door| {
                                        door.is_actor_authorized(
                                            direct,
                                            &auth_info,
                                            true,
                                            allow_leave_map,
                                        )
                                    })
                                    .unwrap_or(false);
                                if !authorized {
                                    tracing::debug!(
                                        entity = ?owner,
                                        door = %door_idx,
                                        ?direct,
                                        "PassDoor: actor not authorized"
                                    );
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                    continue;
                                }

                                // ── Lift-sector authorization ──
                                // Lift doors (wall/ladder
                                // restrictions) get a separate gate.
                                {
                                    let lift_sector_in = self
                                        .mission_script
                                        .as_mut()
                                        .and_then(|s| s.game_host_mut())
                                        .and_then(|h| h.doors.get(usize::from(door_idx)))
                                        .and_then(|d| match d.door_type {
                                            crate::gate::DoorType::LiftHigh
                                            | crate::gate::DoorType::LiftLow
                                            | crate::gate::DoorType::LiftHighCrenel => {
                                                Some(d.sector_in)
                                            }
                                            _ => None,
                                        });
                                    if let Some(sector_in) = lift_sector_in {
                                        let lift_ok = self
                                            .grid_sector_by_number(sector_in)
                                            .and_then(|gs| gs.lift_type)
                                            .map(|lt| lt.is_actor_authorized(&auth_info))
                                            .unwrap_or(true);
                                        if !lift_ok {
                                            tracing::debug!(
                                                entity = ?owner,
                                                door = %door_idx,
                                                ?direct,
                                                "PassDoor: actor not authorized \
                                                 for lift type"
                                            );
                                            self.sequence_manager
                                                .element_impossible(seq_id, elem_idx);
                                            continue;
                                        }
                                    }
                                }

                                // C++ Translate(RHCOMMAND_PASS_DOOR) calls
                                // mpSprite->SetAntiCollisionOn(false) before expanding the
                                // door-pass order chain.
                                if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                                {
                                    entity.position_iface_mut().set_anti_collision_on(false);
                                }

                                // Build the full step chain.  Forward
                                // the movement flags so the animation
                                // picker can pick the
                                // RUNNING_WITH_SWORD variant on fast
                                // moves.
                                let door_pass =
                                    self.build_door_pass(owner, door_idx, direct, *flags);

                                match door_pass {
                                    Some(built) => {
                                        let crate::engine::door_pass::BuiltDoorPass {
                                            pass: mut dp,
                                            post_chain_action_recursive,
                                        } = built;
                                        // Apply
                                        // `SetActionRecursive(WALKING_CROUCHED)`
                                        // to the PassDoor sequence
                                        // element so follow-up orders
                                        // read the crouched action.
                                        // See `build_door_pass` for
                                        // the gate conditions (PC +
                                        // non-direct ladder / wall +
                                        // forced-crouch exit sector).
                                        if let Some(override_action) = post_chain_action_recursive {
                                            self.sequence_manager.set_action_recursive(
                                                seq_id,
                                                elem_idx,
                                                override_action,
                                            );
                                        }
                                        // Pop the first Walk step and start it.
                                        let first_walk = dp.steps.pop_front();
                                        if let Some(crate::element::DoorPassStep::Walk {
                                            destination,
                                            action,
                                            reverse,
                                            compute_direction,
                                            tolerance,
                                        }) = &first_walk
                                        {
                                            // Store the animation from this Walk step
                                            // so tick_entity_movement can use it.
                                            dp.current_action = *action;
                                            dp.current_reverse = *reverse;

                                            // Push the first Walk step's
                                            // destination as a walking order on
                                            // the PassDoor element's queue —
                                            // the Move driver reads the current
                                            // order's `target_x/y` as its goal.
                                            let order_id = self.alloc_order_id();
                                            if let Some(elem) = self
                                                .sequence_manager
                                                .get_element_mut(seq_id, elem_idx)
                                            {
                                                let mut order = crate::order::Order::new(
                                                    *action,
                                                    destination.x,
                                                    destination.y,
                                                    order_id,
                                                );
                                                order.reverse = *reverse;
                                                order.compute_direction = *compute_direction;
                                                order.tolerance = *tolerance;
                                                elem.push_order(order);
                                            }
                                            if let Some(Some(entity)) =
                                                self.entities.get_mut(owner.0 as usize)
                                                && let Some(actor) = entity.actor_data_mut()
                                            {
                                                // Derive action state from the
                                                // movement element's animation.
                                                actor.action_state = match *action {
                                                    crate::order::OrderType::WalkingWithSword => {
                                                        crate::element::ActionState::MovingSword
                                                    }
                                                    crate::order::OrderType::RunningWithSword => {
                                                        crate::element::ActionState::MovingFastSword
                                                    }
                                                    crate::order::OrderType::RunningUpright => {
                                                        crate::element::ActionState::MovingFast
                                                    }
                                                    _ => crate::element::ActionState::Moving,
                                                };
                                                actor.active_movement =
                                                    ActiveMovement::new(seq_id, elem_idx);
                                                actor.passing_door_directly = direct;
                                                actor.active_door_pass = Some(dp);
                                                actor.sequence_element_started = true;
                                            }
                                            self.apply_door_pass_continue_state(owner, *action);
                                            tracing::debug!(
                                                entity = ?owner,
                                                door = %door_idx,
                                                ?direct,
                                                "PassDoor: started multi-step door pass"
                                            );
                                        } else {
                                            tracing::warn!(
                                                entity = ?owner,
                                                "PassDoor: no Walk step in chain"
                                            );
                                            self.sequence_manager
                                                .element_impossible(seq_id, elem_idx);
                                            continue;
                                        }
                                    }
                                    None => {
                                        tracing::warn!(
                                            entity = ?owner,
                                            door = %door_idx,
                                            "PassDoor: failed to build step chain"
                                        );
                                        self.sequence_manager.element_impossible(seq_id, elem_idx);
                                        continue;
                                    }
                                }
                            }
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }
                        // ── CHANGE_POSITION ────────────────────────
                        // Instant teleport to a new position.
                        Command::ChangePosition => {
                            if let crate::sequence::SequenceElementData::Movement {
                                destination,
                                sector,
                                direction,
                                ..
                            } = &elem.data
                            {
                                let dest = *destination;
                                let tgt_sector = *sector;
                                let tgt_direction = *direction;

                                // Verify actor is in expected sector
                                let actor_sector = self
                                    .get_entity(owner)
                                    .and_then(|e| e.element_data().sector());

                                if tgt_sector.is_some() && actor_sector != tgt_sector {
                                    self.sequence_manager.element_interrupted(
                                        seq_id,
                                        elem_idx,
                                        crate::sequence::CascadeFlags::NEXT_LEVEL,
                                    );
                                    continue;
                                }

                                if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                                {
                                    let el = entity.element_data_mut();
                                    el.set_position_map(crate::element::Point2D {
                                        x: dest.x,
                                        y: dest.y,
                                    });
                                    // `SetDirectionInstantly` from the
                                    // element's direction field so a
                                    // ChangePosition can rotate the
                                    // actor in the same step.
                                    el.set_direction_instantly(tgt_direction);
                                    // The original `ComputePositionAll`
                                    // reprojects the sprite's world-
                                    // space caches.  `set_position_map`
                                    // already wrote the 2D projection
                                    // used by the renderer / pathfinder,
                                    // and the sprite's 3D caches are
                                    // derived on read, so no extra
                                    // reproject call is needed here —
                                    // matches the behaviour of the
                                    // other `set_position_map` call
                                    // sites (nets.rs:684,
                                    // script.rs:2573).
                                }
                            }
                            self.sequence_manager.element_terminated(seq_id, elem_idx);
                        }
                        // ── ASSERT_POSITION ────────────────────────
                        // Check actor is at expected position/sector.
                        Command::AssertPosition => {
                            if let crate::sequence::SequenceElementData::Movement {
                                destination,
                                sector,
                                tolerance,
                                ..
                            } = &elem.data
                            {
                                let dest = *destination;
                                let tgt_sector = *sector;
                                let tol = *tolerance + 5.0;

                                if tgt_sector.is_none() {
                                    // Position check
                                    let pos = self
                                        .get_entity(owner)
                                        .map(|e| e.element_data().position_map())
                                        .unwrap_or_default();
                                    let dx = pos.x - dest.x;
                                    let dy = pos.y - dest.y;
                                    if dx.abs().max(dy.abs()) >= tol {
                                        self.sequence_manager.element_interrupted(
                                            seq_id,
                                            elem_idx,
                                            crate::sequence::CascadeFlags::NEXT_LEVEL,
                                        );
                                    } else {
                                        self.sequence_manager.element_terminated(seq_id, elem_idx);
                                    }
                                } else {
                                    // Sector check
                                    let actor_sector = self
                                        .get_entity(owner)
                                        .and_then(|e| e.element_data().sector());
                                    if actor_sector != tgt_sector {
                                        self.sequence_manager.element_interrupted(
                                            seq_id,
                                            elem_idx,
                                            crate::sequence::CascadeFlags::NEXT_LEVEL,
                                        );
                                    } else {
                                        self.sequence_manager.element_terminated(seq_id, elem_idx);
                                    }
                                }
                            } else {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                            }
                        }
                        // ── WAIT_FREE_LIFT ──────────────────────
                        // Wait until the lift sector is authorized to
                        // be entered in this direction, then proceed:
                        //   DOOR_LIFT_HIGH → go downwards → authorized-downwards
                        //   DOOR_LIFT_LOW  → go upwards   → authorized-upwards
                        // Inserted before PASS_DOOR for ladder lifts.
                        //
                        // The authorization check decrements the
                        // lift's wait-time cooldown while blocked and
                        // allows a second actor to ride the lift in
                        // the same direction as the first.
                        Command::WaitFreeLift => {
                            // Resolve the gate → destination sector
                            // (the door's `sector_in` is the lift
                            // shaft).
                            let gate_info =
                                if let crate::sequence::SequenceElementData::Movement {
                                    gate_id: Some(di),
                                    ..
                                } = &elem.data
                                {
                                    self.mission_script
                                        .as_mut()
                                        .and_then(|s| s.game_host_mut())
                                        .and_then(|h| h.doors.get(usize::from(*di)))
                                        .map(|d| {
                                            (
                                                d.sector_in,
                                                matches!(
                                                    d.door_type,
                                                    crate::gate::DoorType::LiftHigh
                                                        | crate::gate::DoorType::LiftHighCrenel
                                                ),
                                            )
                                        })
                                } else {
                                    None
                                };
                            let grid_idx = gate_info.and_then(|(sn, _)| {
                                self.fast_grid.level.sector_number_map.get(&sn).copied()
                            });
                            let is_high = gate_info.map(|(_, h)| h).unwrap_or(false);
                            // `is_authorized_downwards` /
                            // `is_authorized_upwards` decrement
                            // `wait_time` as a side effect when the
                            // lift is on cooldown.
                            let authorised = match grid_idx {
                                Some(idx) => {
                                    let lift = self.fast_grid.lift_state_mut(idx as u32);
                                    if is_high {
                                        lift.is_authorized_downwards()
                                    } else {
                                        lift.is_authorized_upwards()
                                    }
                                }
                                None => true,
                            };

                            if authorised {
                                // Lift is free in the entering direction —
                                // mark occupancy and proceed.
                                // `set_occupied_*` increments
                                // occupants, flips the direction flag,
                                // and sets the wait_time cooldown
                                // (100 for downwards, 80 for upwards).
                                if let Some((sn, _)) = gate_info {
                                    if let Some(idx) = grid_idx {
                                        let lift = self.fast_grid.lift_state_mut(idx as u32);
                                        if is_high {
                                            lift.set_occupied_downwards(true);
                                        } else {
                                            lift.set_occupied_upwards(true);
                                        }
                                    }
                                    // Record the climb on the actor so
                                    // translate_ladder_wall_fall can free the
                                    // lift if the climber is shoved off before
                                    // reaching the other door.  `is_high` means
                                    // the actor entered at the top and is
                                    // climbing downwards, so `upwards = !is_high`.
                                    if let Some(entity) = self.get_entity_mut(owner)
                                        && let Some(actor) = entity.actor_data_mut()
                                    {
                                        actor.active_lift = Some(crate::element::ActiveLiftClimb {
                                            sector_number: u16::from(sn),
                                            upwards: !is_high,
                                        });
                                    }
                                }
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                            } else {
                                // Still occupied or cooldown active —
                                // keep waiting; the authorization
                                // check already decremented
                                // `wait_time` above.
                                self.sequence_manager.element_in_progress(seq_id, elem_idx);
                            }
                        }
                        // ── Sword strike commands ────────────────
                        Command::SwordstrikeThrustA
                        | Command::SwordstrikeThrustB
                        | Command::SwordstrikeThrustC
                        | Command::SwordstrikeThrustD
                        | Command::SwordstrikeThrustE
                        | Command::SwordstrikeThrustF
                        | Command::SwordstrikeThrustG
                        | Command::SwordstrikeThrustH
                        | Command::SwordstrikeThrustI => {
                            let strike = match elem.command {
                                Command::SwordstrikeThrustA => crate::weapons::SwordStrike::A,
                                Command::SwordstrikeThrustB => crate::weapons::SwordStrike::B,
                                Command::SwordstrikeThrustC => crate::weapons::SwordStrike::C,
                                Command::SwordstrikeThrustD => crate::weapons::SwordStrike::D,
                                Command::SwordstrikeThrustE => crate::weapons::SwordStrike::E,
                                Command::SwordstrikeThrustF => crate::weapons::SwordStrike::F,
                                Command::SwordstrikeThrustG => crate::weapons::SwordStrike::G,
                                Command::SwordstrikeThrustH => crate::weapons::SwordStrike::H,
                                Command::SwordstrikeThrustI => crate::weapons::SwordStrike::I,
                                _ => unreachable!(),
                            };
                            let target = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            match target {
                                Some(target_id) => {
                                    self.dispatch_sword_strike(
                                        assets, owner, target_id, strike, seq_id, elem_idx,
                                    );
                                }
                                None => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }

                        // ── Swordfight enter/quit ───────────────
                        Command::EnterSwordfight | Command::PrepareSwordfight => {
                            let opponent = match elem.get_property(crate::sequence::Field::Opponent)
                            {
                                Some(crate::sequence::FieldValue::Element(id)) => Some(*id),
                                _ => None,
                            };
                            self.dispatch_enter_swordfight(
                                assets, owner, opponent, seq_id, elem_idx,
                            );
                        }
                        Command::QuitSwordfight => {
                            self.dispatch_quit_swordfight(owner, seq_id, elem_idx);
                        }

                        // ── Parry commands ──────────────────────
                        Command::ParrySword => {
                            self.dispatch_parry_sword(owner, false, seq_id, elem_idx);
                        }
                        Command::ParrySwordLow => {
                            self.dispatch_parry_sword(owner, true, seq_id, elem_idx);
                        }
                        Command::StopParrySword => {
                            self.dispatch_stop_parry(owner, seq_id, elem_idx);
                        }

                        // ── Damage reception commands ───────────
                        Command::ReceiveSwordDamage
                        | Command::ReceiveDamage
                        | Command::ReceiveArrowDamage
                        | Command::ReceiveStoneDamage
                        | Command::ReceiveHitDamage
                        | Command::ReceiveMobileDamage
                        | Command::ReceiveNet => {
                            self.dispatch_receive_damage(assets, owner, seq_id, elem_idx);
                        }

                        // ── Shoulder-fall sub-sequence ──────────
                        // Launched by `translate_shoulder_damage` on
                        // the carrier/carried partner when shoulder-
                        // damage lands on the other side of the carry.
                        Command::Fall => {
                            self.dispatch_fall(owner, seq_id, elem_idx);
                        }

                        // ── NPC head-turn / lean-out commands ────
                        // Insert a Looking{Left,Right}[Alerted] or
                        // TransitionWaitingAlertedLeaningOut order on
                        // the actor's queue, then stay in-progress
                        // until the sprite reaches DONE.  Terminating
                        // the element immediately (as the code did
                        // before) let `LOOK_LEFT_RIGHT` sequences
                        // advance to the second command before the
                        // first animation ran, so the second booking
                        // overwrote the first and only one of the
                        // two head turns played.
                        Command::LookLeft | Command::LookRight | Command::LeanOut => {
                            // Push a `LookingLeft[Alerted]` /
                            // `LookingRight[Alerted]` /
                            // `TransitionWaitingAlertedLeaningOut`
                            // order onto the sequence element's queue
                            // and mark the actor's `active_ai_anim`
                            // so the sprite plays the head-turn
                            // animation.
                            //
                            // The order queue is what `refresh_view` reads
                            // through `current_order_for_actor(npc)` to
                            // decide whether to hold `eye_status` at
                            // `LookToTheLeft`/`Right`; without the queue
                            // entry, `refresh_view` can't validate the
                            // look-sidewards eye status and snaps it back
                            // to `LookForward`, which means the vision
                            // cone never rotates even though the sprite
                            // animation plays.  So both sides are needed.
                            let order_type =
                                if let Some(Some(entity)) = self.entities.get(owner.0 as usize) {
                                    let attentive = entity.enemy_ai().is_some_and(|e| e.attentive);
                                    let ot = match elem.command {
                                    Command::LookLeft => {
                                        if attentive {
                                            crate::order::OrderType::LookingLeftAlerted
                                        } else {
                                            crate::order::OrderType::LookingLeft
                                        }
                                    }
                                    Command::LookRight => {
                                        if attentive {
                                            crate::order::OrderType::LookingRightAlerted
                                        } else {
                                            crate::order::OrderType::LookingRight
                                        }
                                    }
                                    _ => {
                                        crate::order::OrderType::TransitionWaitingAlertedLeaningOut
                                    }
                                };
                                    Some(ot)
                                } else {
                                    None
                                };
                            let queued = if let Some(ot) = order_type {
                                let owner_alive = self.get_entity(owner).is_some();
                                if owner_alive {
                                    self.push_new_order(seq_id, elem_idx, ot, 0.0, 0.0);
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            if queued {
                                self.sequence_manager.element_in_progress(seq_id, elem_idx);
                            } else {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                            }
                        }

                        // ── Attentive-mode transitions ───────────
                        Command::EnterAttentiveMode
                        | Command::LeaveAttentiveMode
                        | Command::LeaveAttentiveModeOfficer => {
                            let posture_after = elem.posture_after_transition;
                            let queued_anim = self.dispatch_attentive_transition(
                                owner,
                                elem.command,
                                posture_after,
                                seq_id,
                                elem_idx,
                            );
                            if queued_anim {
                                self.sequence_manager.element_in_progress(seq_id, elem_idx);
                            } else {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                            }
                        }

                        // ── Wasp sting ─────────────────────────
                        Command::ReceiveWaspSting => {
                            self.dispatch_receive_wasp_sting(assets, owner, seq_id, elem_idx);
                        }

                        // ── Stealth posture commands ────────────
                        Command::CrouchDown
                        | Command::CrouchUp
                        | Command::EnterBeggar
                        | Command::LeaveBeggar
                        | Command::EnterHelpingClimb
                        | Command::LeaveHelpingClimb
                        | Command::LeaveSpy
                        | Command::LeaveTree => {
                            self.dispatch_stealth_command(
                                assets,
                                owner,
                                elem.command,
                                seq_id,
                                elem_idx,
                            );
                        }

                        // ── Shield commands ─────────────────────
                        Command::RaiseShield => {
                            self.dispatch_raise_shield(owner, seq_id, elem_idx);
                        }
                        Command::RaiseShieldInstantly => {
                            self.dispatch_raise_shield_instantly(owner, seq_id, elem_idx);
                        }
                        Command::LowerShield => {
                            self.dispatch_lower_shield(owner, seq_id, elem_idx);
                        }
                        Command::ParryShield => {
                            self.dispatch_parry_shield(owner, seq_id, elem_idx);
                        }
                        // ── Bow equip / raise / lower ───────────
                        //
                        // C++ RHElementActorHuman::Translate appends
                        // these bow animation orders from the command
                        // body itself. Some command profiles may have
                        // already queued transition orders before
                        // translate; when they have not, push the
                        // command's own orders here.
                        Command::EquipBow
                        | Command::EquipBowDown
                        | Command::UnequipBow
                        | Command::RaiseBow
                        | Command::LowerBow => {
                            let command_body_already_queued = self
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .is_some_and(|e| {
                                    e.orders.iter().any(|o| {
                                        use crate::order::OrderType as OT;
                                        match elem.command {
                                            Command::EquipBow => matches!(
                                                o.order_type,
                                                OT::TransitionEquipBow
                                                    | OT::TransitionEquipBowAnonymous
                                            ),
                                            Command::EquipBowDown => {
                                                o.order_type == OT::TransitionLoweringBowLeaningOut
                                            }
                                            Command::UnequipBow => matches!(
                                                o.order_type,
                                                OT::TransitionUnloadBow
                                                    | OT::TransitionUnloadBowAnonymous
                                                    | OT::TransitionUnequipBow
                                                    | OT::TransitionUnequipBowAnonymous
                                            ),
                                            Command::RaiseBow => matches!(
                                                o.order_type,
                                                OT::TransitionRaisingBow
                                                    | OT::TransitionRaisingBowAnonymous
                                            ),
                                            Command::LowerBow => matches!(
                                                o.order_type,
                                                OT::TransitionLoweringBow
                                                    | OT::TransitionLoweringBowAnonymous
                                            ),
                                            _ => false,
                                        }
                                    })
                                });
                            let append_command_body = !command_body_already_queued;
                            if append_command_body {
                                let owner_entity = self.get_entity(owner).unwrap_or_else(|| {
                                    panic!("bow command owner missing: {owner:?}")
                                });
                                let posture = owner_entity.element_data().posture;
                                let owner_action_state = owner_entity
                                    .actor_data()
                                    .map(|actor| actor.action_state)
                                    .unwrap_or_else(|| {
                                        panic!("bow command owner missing actor data: {owner:?}")
                                    });
                                if matches!(elem.command, Command::EquipBow | Command::EquipBowDown)
                                    && owner_action_state.is_bow()
                                {
                                    // C++ `Translate(EQUIP_BOW*)` terminates
                                    // non-transition command bodies when the
                                    // actor is already aiming with the bow.
                                    self.sequence_manager.element_terminated(seq_id, elem_idx);
                                    continue;
                                }
                                let anonymous = posture == crate::element::Posture::AnonymousArcher;
                                let push = |engine: &mut EngineInner,
                                            ot: crate::order::OrderType,
                                            x: f32,
                                            y: f32| {
                                    let id = engine.alloc_order_id();
                                    let mut order = crate::order::Order::new(ot, x, y, id);
                                    order.compute_direction = false;
                                    engine
                                        .sequence_manager
                                        .push_order_on(seq_id, elem_idx, order);
                                };
                                let target_xy = self
                                    .sequence_manager
                                    .get_element(seq_id, elem_idx)
                                    .and_then(|e| e.orders.back())
                                    .map(|o| (o.target_x, o.target_y))
                                    .unwrap_or((0.0, 0.0));

                                use crate::element::ActionState;
                                use crate::order::OrderType;
                                match elem.command {
                                    Command::EquipBow => {
                                        if anonymous {
                                            push(
                                                self,
                                                OrderType::TransitionEquipBowAnonymous,
                                                0.0,
                                                0.0,
                                            );
                                            push(
                                                self,
                                                OrderType::TransitionLoadingBowAnonymous,
                                                0.0,
                                                0.0,
                                            );
                                        } else {
                                            push(self, OrderType::TransitionEquipBow, 0.0, 0.0);
                                            push(self, OrderType::TransitionLoadingBow, 0.0, 0.0);
                                        }
                                        if let Some(elem) =
                                            self.sequence_manager.get_element_mut(seq_id, elem_idx)
                                        {
                                            elem.action_state_after_transition =
                                                ActionState::AimingWithBow;
                                        }
                                    }
                                    Command::EquipBowDown => {
                                        push(self, OrderType::TransitionEquipBow, 0.0, 0.0);
                                        push(self, OrderType::TransitionLoadingBow, 0.0, 0.0);
                                        push(
                                            self,
                                            OrderType::TransitionLoweringBowLeaningOut,
                                            0.0,
                                            0.0,
                                        );
                                        if let Some(elem) =
                                            self.sequence_manager.get_element_mut(seq_id, elem_idx)
                                        {
                                            elem.action_state_after_transition =
                                                ActionState::AimingWithBowDown;
                                        }
                                    }
                                    Command::UnequipBow => {
                                        let (x, y) = target_xy;
                                        if anonymous {
                                            push(
                                                self,
                                                OrderType::TransitionUnloadBowAnonymous,
                                                x,
                                                y,
                                            );
                                            push(
                                                self,
                                                OrderType::TransitionUnequipBowAnonymous,
                                                x,
                                                y,
                                            );
                                        } else {
                                            push(self, OrderType::TransitionUnloadBow, x, y);
                                            push(self, OrderType::TransitionUnequipBow, x, y);
                                        }
                                        if let Some(elem) =
                                            self.sequence_manager.get_element_mut(seq_id, elem_idx)
                                        {
                                            elem.action_state_after_transition =
                                                ActionState::Waiting;
                                        }
                                    }
                                    Command::RaiseBow => {
                                        if anonymous {
                                            push(
                                                self,
                                                OrderType::TransitionRaisingBowAnonymous,
                                                0.0,
                                                0.0,
                                            );
                                        } else {
                                            push(self, OrderType::TransitionRaisingBow, 0.0, 0.0);
                                        }
                                        if let Some(elem) =
                                            self.sequence_manager.get_element_mut(seq_id, elem_idx)
                                        {
                                            elem.action_state_after_transition =
                                                ActionState::AimingWithBowUp;
                                        }
                                    }
                                    Command::LowerBow => {
                                        if anonymous {
                                            push(
                                                self,
                                                OrderType::TransitionLoweringBowAnonymous,
                                                0.0,
                                                0.0,
                                            );
                                        } else {
                                            push(self, OrderType::TransitionLoweringBow, 0.0, 0.0);
                                        }
                                        if let Some(elem) =
                                            self.sequence_manager.get_element_mut(seq_id, elem_idx)
                                        {
                                            elem.action_state_after_transition =
                                                ActionState::AimingWithBow;
                                        }
                                    }
                                    _ => unreachable!(),
                                }
                            }

                            let has_orders = self
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .is_some_and(|e| !e.orders.is_empty());
                            if has_orders {
                                self.sequence_manager.element_in_progress(seq_id, elem_idx);
                            } else {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                            }
                        }
                        // ── Hide behind shield ──────────────────
                        //
                        // 1. Holder must be holding-shield (HOLDING/
                        //    MOVING/PARRYING) AND not currently
                        //    protecting anyone.  Otherwise → INTERRUPTED
                        //    (note: this is stricter than the
                        //    validity gate, which permits
                        //    `holder.shield_protected == self`).
                        // 2. If the element's posture-after-transition is
                        //    not Crouched, prepend a TRANSITION_CROUCHING_DOWN
                        //    order so the actor crouches before hiding.
                        // 3. Push the HIDING_BEHIND_SHIELD non-animation
                        //    order with the shield holder as antagonist.
                        Command::HideBehindShield => {
                            let antagonist = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            let posture_after = elem.posture_after_transition;
                            let Some(holder) = antagonist else {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let (is_holding, holder_protected) = self
                                .get_entity(holder)
                                .map(|e| {
                                    let h = e
                                        .actor_data()
                                        .map(|a| a.action_state.is_shield())
                                        .unwrap_or(false);
                                    let p = e.pc_data().and_then(|pc| pc.shield_protected);
                                    (h, p)
                                })
                                .unwrap_or((false, None));
                            if !is_holding || holder_protected.is_some() {
                                self.sequence_manager.element_interrupted(
                                    seq_id,
                                    elem_idx,
                                    crate::sequence::CascadeFlags::NEXT_LEVEL,
                                );
                                continue;
                            }
                            if posture_after != crate::element::Posture::Crouched {
                                let id = self.alloc_order_id();
                                let mut order = crate::order::Order::new(
                                    crate::order::OrderType::TransitionCrouchingDown,
                                    0.0,
                                    0.0,
                                    id,
                                );
                                order.compute_direction = false;
                                self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                            }
                            let id = self.alloc_order_id();
                            let mut order = crate::order::Order::new(
                                crate::order::OrderType::HidingBehindShield,
                                0.0,
                                0.0,
                                id,
                            )
                            .with_antagonist(holder);
                            order.compute_direction = false;
                            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }

                        // ── Other sword-related commands ────────
                        Command::SwordstrikeDown => {
                            let antagonist = self
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .and_then(|elem| match &elem.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => *antagonist,
                                    _ => None,
                                });
                            let Some(target) = antagonist else {
                                tracing::warn!(
                                    ?seq_id,
                                    elem_idx,
                                    "SwordstrikeDown missing antagonist"
                                );
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let (tx, ty, dir) =
                                match (self.get_entity(owner), self.get_entity(target)) {
                                    (Some(owner_entity), Some(target_entity)) => {
                                        let owner_pos = owner_entity.element_data().position_map();
                                        let target_pos =
                                            target_entity.element_data().position_map();
                                        let dir =
                                            crate::position_interface::vector_to_sector_0_to_15(
                                                target_pos.x - owner_pos.x,
                                                target_pos.y - owner_pos.y,
                                            );
                                        (target_pos.x, target_pos.y, dir)
                                    }
                                    _ => {
                                        tracing::warn!(
                                            ?owner,
                                            ?target,
                                            "SwordstrikeDown owner or target missing"
                                        );
                                        self.sequence_manager.element_impossible(seq_id, elem_idx);
                                        continue;
                                    }
                                };
                            if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize) {
                                entity.element_data_mut().set_direction_instantly(dir);
                                if let Some(actor) = entity.actor_data_mut() {
                                    actor.clear_path();
                                }
                            }
                            let mut order = crate::order::Order::new(
                                crate::order::OrderType::StrikingDownSword,
                                tx,
                                ty,
                                self.alloc_order_id(),
                            )
                            .with_antagonist(target);
                            order.compute_direction = false;
                            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }
                        Command::GetKilledAtBottom => {
                            let killer = self
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .and_then(|elem| match elem.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => antagonist,
                                    _ => None,
                                });
                            let Some(Some(victim)) = self.entities.get_mut(owner.0 as usize) else {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let damage = victim
                                .human_and_life_points_mut()
                                .map(|(_, lp)| (*lp).max(0) as u16);
                            let Some(damage) = damage else {
                                tracing::warn!(
                                    ?owner,
                                    ?killer,
                                    "GetKilledAtBottom owner is not a human"
                                );
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let max_life_points = match victim {
                                crate::element::Entity::Pc(_) => crate::combat::LIFEPOINTS_PC,
                                crate::element::Entity::Soldier(s) => {
                                    s.soldier.cached_max_life_points
                                }
                                crate::element::Entity::Civilian(_) => 100,
                                _ => 100,
                            };
                            if let Some((_, lp)) = victim.human_and_life_points_mut() {
                                crate::combat::get_wounded(
                                    lp,
                                    damage,
                                    false,
                                    max_life_points,
                                    false,
                                );
                            }
                            let is_rider = matches!(
                                victim,
                                crate::element::Entity::Soldier(s) if s.soldier.rider
                            );
                            if is_rider {
                                let anim = victim
                                    .actor_data()
                                    .map(|actor| {
                                        let action_state = actor.action_state;
                                        if action_state.is_sword()
                                            || action_state == crate::element::ActionState::Menacing
                                        {
                                            crate::order::OrderType::DyingSword
                                        } else if action_state.is_bow() {
                                            crate::order::OrderType::DyingBow
                                        } else {
                                            crate::order::OrderType::DyingUpright
                                        }
                                    })
                                    .unwrap_or(crate::order::OrderType::DyingUpright);
                                self.push_new_order(seq_id, elem_idx, anim, 0.0, 0.0);
                                self.sequence_manager.element_in_progress(seq_id, elem_idx);
                            } else {
                                if victim.is_dead() {
                                    victim.set_posture(crate::element::Posture::DeadBack);
                                }
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                            }
                        }
                        // SwordstrikeTired pushes a `BeingWeakSword`
                        // animation order; the order is consumed by
                        // `do_next_order` and (on a soldier)
                        // `apply_combat_injury_side_effect`
                        // dispatches `EventAfterCombatInjury` so the
                        // AI can resume the fight.
                        Command::SwordstrikeTired => {
                            if self.get_entity(owner).is_some() {
                                self.push_new_order(
                                    seq_id,
                                    elem_idx,
                                    crate::order::OrderType::BeingWeakSword,
                                    0.0,
                                    0.0,
                                );
                                self.sequence_manager.element_in_progress(seq_id, elem_idx);
                            } else {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                            }
                        }
                        // ── Smalltalk strikes / parries (Wait priority) ─
                        // The smalltalk strike / parry commands carry
                        // a single cosmetic animation order.  Drive
                        // it via `active_ai_anim` so completion
                        // terminates the element naturally AND
                        // arbitration (Wait vs anything else →
                        // InterruptCurrent) can tear it down cleanly
                        // when a real action arrives.
                        Command::SwordstrikeSmalltalkLeft
                        | Command::SwordstrikeSmalltalkRight
                        | Command::ParrySmalltalkLeft
                        | Command::ParrySmalltalkRight => {
                            let antagonist = self
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .and_then(|elem| match elem.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => antagonist,
                                    _ => None,
                                });
                            let owner_higher = antagonist
                                .and_then(|id| {
                                    let owner_z = self
                                        .get_entity(owner)
                                        .map(|e| e.element_data().position().z)?;
                                    let opponent_z = self
                                        .get_entity(id)
                                        .map(|e| e.element_data().position().z)?;
                                    Some(owner_z >= opponent_z + 20.0)
                                })
                                .unwrap_or(false);
                            let order_type = match cmd {
                                Command::SwordstrikeSmalltalkLeft if owner_higher => {
                                    crate::order::OrderType::StrikingLowLeftSmalltalk
                                }
                                Command::SwordstrikeSmalltalkLeft => {
                                    crate::order::OrderType::StrikingLeftSmalltalk
                                }
                                Command::SwordstrikeSmalltalkRight if owner_higher => {
                                    crate::order::OrderType::StrikingLowRightSmalltalk
                                }
                                Command::SwordstrikeSmalltalkRight => {
                                    crate::order::OrderType::StrikingRightSmalltalk
                                }
                                Command::ParrySmalltalkLeft if owner_higher => {
                                    crate::order::OrderType::ParryingLowLeftSmalltalk
                                }
                                Command::ParrySmalltalkLeft => {
                                    crate::order::OrderType::ParryingLeftSmalltalk
                                }
                                Command::ParrySmalltalkRight if owner_higher => {
                                    crate::order::OrderType::ParryingLowRightSmalltalk
                                }
                                Command::ParrySmalltalkRight => {
                                    crate::order::OrderType::ParryingRightSmalltalk
                                }
                                _ => unreachable!(),
                            };
                            // Guard: skip if a higher-priority action is
                            // already running for this actor (combat).
                            let blocked = self
                                .get_entity(owner)
                                .and_then(|e| e.actor_data())
                                .map(|a| a.active_melee.is_active())
                                .unwrap_or(true);
                            if !blocked {
                                self.push_new_order(seq_id, elem_idx, order_type, 0.0, 0.0);
                                self.sequence_manager.element_in_progress(seq_id, elem_idx);
                            } else {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                            }
                        }

                        // ── Provoke (taunt) ─────────────────────
                        // Say `ProvokesCombat` and queue a `Provoking`
                        // animation order (with `compute_direction =
                        // false`).  The animation is consumed via
                        // `active_ai_anim` tied to the sequence
                        // element; its START hook in
                        // `melee::process_pc_combat_anim_speech`
                        // fires `HERO_PROVOKE_OPPONENT` for PCs.
                        Command::Provoke => {
                            if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                                && let Some(ai) = entity.ai_controller_mut()
                            {
                                ai.say(crate::ai::Remark::ProvokesCombat);
                            }
                            // Append the order to the sequence
                            // element's queue.
                            let mut order = crate::order::Order::new(
                                crate::order::OrderType::Provoking,
                                0.0,
                                0.0,
                                self.alloc_order_id(),
                            );
                            order.compute_direction = false;
                            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }
                        Command::Fainted => {
                            // Queue the faint/knockout animation on the owning
                            // element (the element is terminated immediately
                            // below — the queued order is consumed by the
                            // animation driver before cleanup).
                            self.push_new_order(
                                seq_id,
                                elem_idx,
                                crate::order::OrderType::BeingUnconsciousSword,
                                0.0,
                                0.0,
                            );
                            self.sequence_manager.element_terminated(seq_id, elem_idx);
                        }
                        Command::Recover | Command::StandUp => {
                            // STAND_UP picks the standup animation by
                            // current action state
                            // (`StandingUp[Sword|Bow]`) and inserts
                            // it as an order.  When the launcher
                            // pre-pushed orders (e.g.
                            // `handle_post_concussion` chains
                            // standup + BeingStunnedSword), use
                            // those — the front order plays first
                            // and `do_next_order` chains through the
                            // rest.
                            let already_queued = self
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .map(|e| !e.orders.is_empty())
                                .unwrap_or(false);
                            if !already_queued {
                                let standing_up = match self
                                    .entities
                                    .get(owner.0 as usize)
                                    .and_then(|slot| slot.as_ref())
                                    .and_then(|entity| entity.actor_data())
                                {
                                    Some(actor) => {
                                        let action_state = actor.action_state;
                                        if action_state.is_sword()
                                            || action_state == crate::element::ActionState::Menacing
                                        {
                                            crate::order::OrderType::StandingUpSword
                                        } else if action_state.is_bow() {
                                            crate::order::OrderType::StandingUpBow
                                        } else {
                                            crate::order::OrderType::StandingUp
                                        }
                                    }
                                    None => {
                                        tracing::warn!(
                                            "StandUp/Recover owner has no actor data; defaulting to StandingUp owner={owner:?} seq_id={seq_id:?} elem_idx={elem_idx}"
                                        );
                                        crate::order::OrderType::StandingUp
                                    }
                                };
                                self.push_new_order(seq_id, elem_idx, standing_up, 0.0, 0.0);
                            }
                            // Pre-pushed orders (e.g. `handle_post_concussion`)
                            // already carry stamped `order_id`s (required
                            // at construction), so no batch fixup is needed.
                            if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize) {
                                entity.set_posture(crate::element::Posture::Upright);
                            }
                            let has_front = self
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .and_then(|e| e.orders.front())
                                .is_some();
                            if has_front {
                                self.sequence_manager.element_in_progress(seq_id, elem_idx);
                            } else {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                            }
                        }
                        Command::WakeUp => {
                            let antagonist = self
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .and_then(|elem| match elem.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => antagonist,
                                    _ => None,
                                });
                            let Some(target_id) = antagonist else {
                                tracing::warn!(
                                    ?owner,
                                    ?seq_id,
                                    elem_idx,
                                    "WakeUp element has no antagonist target"
                                );
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let Some(target_pos) = self
                                .get_entity(target_id)
                                .map(|entity| entity.element_data().position_map())
                            else {
                                tracing::warn!(
                                    ?owner,
                                    ?target_id,
                                    ?seq_id,
                                    elem_idx,
                                    "WakeUp antagonist target is missing"
                                );
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let mut order = crate::order::Order::new(
                                crate::order::OrderType::WakingUp,
                                target_pos.x,
                                target_pos.y,
                                self.alloc_order_id(),
                            )
                            .with_antagonist(target_id);
                            order.compute_direction = false;
                            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }
                        Command::Knee => {
                            // Queue the falling-to-knees animation.
                            self.push_new_order(
                                seq_id,
                                elem_idx,
                                crate::order::OrderType::FallingBackSword,
                                0.0,
                                0.0,
                            );
                            self.sequence_manager.element_terminated(seq_id, elem_idx);
                        }

                        // ── Ability commands ─────────────────────
                        Command::TakeCorpse => {
                            let target = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            match target {
                                Some(target_id) => {
                                    match abilities::begin_carry(
                                        &mut self.entities,
                                        &mut self.sequence_manager,
                                        owner,
                                        target_id,
                                        seq_id,
                                        elem_idx,
                                        &mut self.next_order_id,
                                    ) {
                                        AbilityBeginResult::Started => {
                                            self.sequence_manager
                                                .element_in_progress(seq_id, elem_idx);
                                            // Freeze the target's
                                            // execution, cascading
                                            // the interrupt on its
                                            // current sequence
                                            // element so a postponed
                                            // successor resumes
                                            // cleanly after the carry
                                            // ends.
                                            self.actor_freeze_execution(target_id);
                                            // Inside a building,
                                            // re-select + start hulk
                                            // on the carried target
                                            // flashes the body
                                            // through walls.
                                            self.apply_carry_building_hulk(owner, target_id);
                                        }
                                        AbilityBeginResult::Impossible => {
                                            self.sequence_manager
                                                .element_impossible(seq_id, elem_idx);
                                        }
                                    }
                                }
                                None => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::DropCorpse => {
                            match abilities::begin_drop(
                                &mut self.entities,
                                &mut self.sequence_manager,
                                owner,
                                seq_id,
                                elem_idx,
                                &mut self.next_order_id,
                            ) {
                                AbilityBeginResult::Started => {
                                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                                    // Drop-transition init twin of
                                    // the pickup building flash.
                                    let carried_id = self
                                        .get_entity(owner)
                                        .and_then(|e| e.pc_data())
                                        .and_then(|pc| pc.carried);
                                    if let Some(cid) = carried_id {
                                        // Re-freeze the carried on
                                        // drop init.  The victim is
                                        // normally already frozen
                                        // from the carry, but this
                                        // idempotently re-runs the
                                        // cascade-interrupt so any
                                        // element that slipped onto
                                        // the carried (e.g. a
                                        // script-driven
                                        // `ActionChange`) is
                                        // interrupted.
                                        self.actor_freeze_execution(cid);
                                        self.apply_carry_building_hulk(owner, cid);
                                    }
                                }
                                AbilityBeginResult::Impossible => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::TieCmd => {
                            let target = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            match target {
                                Some(target_id) => {
                                    match abilities::begin_tie(
                                        &mut self.entities,
                                        &mut self.sequence_manager,
                                        owner,
                                        target_id,
                                        seq_id,
                                        elem_idx,
                                        &mut self.next_order_id,
                                    ) {
                                        AbilityBeginResult::Started => {
                                            self.sequence_manager
                                                .element_in_progress(seq_id, elem_idx);
                                        }
                                        AbilityBeginResult::Impossible => {
                                            self.sequence_manager
                                                .element_impossible(seq_id, elem_idx);
                                        }
                                    }
                                }
                                None => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::ClimbDownFromShoulders => {
                            // Owner is the climber; the carrier
                            // (helper) is read from the climber's
                            // `human.carrier` back-reference latched
                            // at climb-up time.
                            let carrier_id = self
                                .get_entity(owner)
                                .and_then(|e| e.human_data())
                                .and_then(|h| h.carrier);
                            match abilities::begin_climb_down_from_shoulders(
                                &mut self.entities,
                                &mut self.sequence_manager,
                                owner,
                                seq_id,
                                elem_idx,
                                &mut self.next_order_id,
                            ) {
                                AbilityBeginResult::Started => {
                                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                                    // Helper is frozen for the
                                    // duration of the climb-down so
                                    // it can't acquire a fresh
                                    // sequence element while playing
                                    // the sync'd
                                    // TRANSITION_HELPING_CLIMBING_DOWN.
                                    if let Some(helper_id) = carrier_id {
                                        self.actor_freeze_execution(helper_id);
                                    }
                                }
                                AbilityBeginResult::Impossible => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::ClimbUpOnShoulders => {
                            // Owner is the climber, antagonist is the
                            // HelpingToClimb helper.
                            let helper = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            let Some(helper_id) = helper else {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            // Disjoint-field obstacle list so the headroom
                            // ray-cast inside `begin_climb_on_shoulders`
                            // can run alongside the `&mut self.entities`
                            // borrow.
                            let obstacles = crate::sight_obstacle::ObstacleList {
                                static_obstacles: assets.static_sight_obstacles.as_slice(),
                                dynamic_obstacles: &self.dynamic_sight_obstacles,
                                static_active: &self.static_sight_obstacle_active,
                            };
                            match abilities::begin_climb_on_shoulders(
                                &mut self.entities,
                                &mut self.sequence_manager,
                                owner,
                                helper_id,
                                seq_id,
                                elem_idx,
                                &mut self.next_order_id,
                                obstacles,
                            ) {
                                crate::abilities::ClimbResult::Started => {
                                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                                    // Helper is frozen for the
                                    // duration of the climb.
                                    self.actor_freeze_execution(helper_id);
                                }
                                crate::abilities::ClimbResult::Impossible => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                                crate::abilities::ClimbResult::NoHeadroom { helper_id } => {
                                    // Low ceiling → helper stands
                                    // back up (LeaveHelpingClimb) and
                                    // the climber's element is
                                    // Impossible.
                                    let leave_elem = crate::sequence::SequenceElement::new(
                                        1,
                                        crate::element::Command::LeaveHelpingClimb,
                                        Some(helper_id),
                                    );
                                    self.launch_element(leave_elem);
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::HealCmd => {
                            let target = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            match target {
                                Some(target_id) => {
                                    if !self.has_ammo(owner, crate::profiles::Action::Heal) {
                                        self.sequence_manager.element_impossible(seq_id, elem_idx);
                                    } else {
                                        match abilities::begin_heal(
                                            &mut self.entities,
                                            &mut self.sequence_manager,
                                            owner,
                                            target_id,
                                            seq_id,
                                            elem_idx,
                                            &mut self.next_order_id,
                                        ) {
                                            AbilityBeginResult::Started => {
                                                self.sequence_manager
                                                    .element_in_progress(seq_id, elem_idx);
                                            }
                                            AbilityBeginResult::Impossible => {
                                                self.sequence_manager
                                                    .element_impossible(seq_id, elem_idx);
                                            }
                                        }
                                    }
                                }
                                None => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::WhistleCmd => {
                            match abilities::begin_whistle(
                                &mut self.entities,
                                &mut self.sequence_manager,
                                owner,
                                seq_id,
                                elem_idx,
                                &mut self.next_order_id,
                            ) {
                                AbilityBeginResult::Started => {
                                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                                }
                                AbilityBeginResult::Impossible => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::EatCmd => {
                            // If eat ammo > 0, push the EATING order;
                            // otherwise terminate.  Eat and Guzzle
                            // share the `num_rations` counter
                            // (pc_status.rs:374-387), so a single
                            // `Action::Eat` lookup covers both.
                            let ammo = self
                                .get_entity(owner)
                                .and_then(|e| match e {
                                    crate::element::Entity::Pc(pc) => {
                                        self.pc_description_for_pc_data(&pc.pc)
                                    }
                                    _ => None,
                                })
                                .map(|d| d.status.get_ammo(crate::profiles::Action::Eat))
                                .unwrap_or(0);
                            if ammo == 0 {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                                continue;
                            }
                            match abilities::begin_eat(
                                &mut self.entities,
                                &mut self.sequence_manager,
                                owner,
                                seq_id,
                                elem_idx,
                                &mut self.next_order_id,
                            ) {
                                AbilityBeginResult::Started => {
                                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                                }
                                AbilityBeginResult::Impossible => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::HitCmd => {
                            let target = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            match target {
                                Some(target_id) => {
                                    match abilities::begin_hit(
                                        &mut self.entities,
                                        &mut self.sequence_manager,
                                        owner,
                                        target_id,
                                        seq_id,
                                        elem_idx,
                                        &mut self.next_order_id,
                                    ) {
                                        AbilityBeginResult::Started => {
                                            self.sequence_manager
                                                .element_in_progress(seq_id, elem_idx);
                                        }
                                        AbilityBeginResult::Impossible => {
                                            self.sequence_manager
                                                .element_impossible(seq_id, elem_idx);
                                        }
                                    }
                                }
                                None => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::StrangleCmd => {
                            let target = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            match target {
                                Some(target_id) => {
                                    match abilities::begin_strangle(
                                        &mut self.entities,
                                        &mut self.sequence_manager,
                                        owner,
                                        target_id,
                                        seq_id,
                                        elem_idx,
                                        &mut self.next_order_id,
                                    ) {
                                        AbilityBeginResult::Started => {
                                            self.sequence_manager
                                                .element_in_progress(seq_id, elem_idx);
                                        }
                                        AbilityBeginResult::Impossible => {
                                            self.sequence_manager
                                                .element_impossible(seq_id, elem_idx);
                                        }
                                    }
                                }
                                None => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::Pay => {
                            // Validate campaign has enough ransom.
                            // The original aborts with the post-walk
                            // validity check if ransom dropped
                            // mid-sequence.  We pre-check on launch;
                            // a race where ransom becomes
                            // insufficient between the click and the
                            // animation is acceptable (next frame's
                            // completion handler would just not
                            // deduct — see PayDone branch).
                            let beggar = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            match beggar {
                                Some(beggar_id) => {
                                    match abilities::begin_pay(
                                        &mut self.entities,
                                        &mut self.sequence_manager,
                                        owner,
                                        beggar_id,
                                        seq_id,
                                        elem_idx,
                                        &mut self.next_order_id,
                                    ) {
                                        AbilityBeginResult::Started => {
                                            // HERO_GIVE_MONEY speech
                                            // cue.
                                            self.hero_speaking(
                                                assets,
                                                owner,
                                                crate::engine::melee::HERO_GIVE_MONEY,
                                            );
                                            self.sequence_manager
                                                .element_in_progress(seq_id, elem_idx);
                                        }
                                        AbilityBeginResult::Impossible => {
                                            self.sequence_manager
                                                .element_impossible(seq_id, elem_idx);
                                        }
                                    }
                                }
                                None => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::ReceivePurse => {
                            match abilities::begin_receive_purse(
                                &mut self.entities,
                                owner,
                                seq_id,
                                elem_idx,
                                &mut self.next_order_id,
                            ) {
                                AbilityBeginResult::Started => {
                                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                                }
                                AbilityBeginResult::Impossible => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::EnterListen => {
                            // Completion is driven by the multi-phase
                            // Listen state machine: begin_listen →
                            // entry transition (tick_abilities) →
                            // CountingDown (ai.rs) → exit transition
                            // (tick_abilities) → ListenDone →
                            // element_terminated in combat.rs.
                            match abilities::begin_listen(
                                &mut self.entities,
                                &assets.profile_manager,
                                &mut self.sequence_manager,
                                owner,
                                seq_id,
                                elem_idx,
                                &mut self.next_order_id,
                            ) {
                                AbilityBeginResult::Started => {
                                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                                }
                                AbilityBeginResult::Impossible => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::LeaveListen => {
                            // Cancel an in-progress Listen by flipping
                            // `listen_phase` to `ExitTransition`.  The
                            // LeaveListen sequence element has no
                            // animation of its own — the still-active
                            // EnterListen ability drives the exit
                            // transition animation — so we terminate
                            // the LeaveListen element immediately.
                            if abilities::begin_leave_listen(
                                &mut self.entities,
                                owner,
                                &mut self.next_order_id,
                            ) {
                                tracing::debug!(
                                    ?owner,
                                    "Listen: LeaveListen flipped phase to ExitTransition"
                                );
                            }
                            self.sequence_manager.element_terminated(seq_id, elem_idx);
                        }
                        Command::DropAmmo => {
                            // Decrement the PC's ammo for the action,
                            // then either merge into an adjacent
                            // just-dropped bonus (same action,
                            // combined quantity ≤ 5) or spawn a fresh
                            // `ElementBonus` at the PC's position.
                            // We skip the TAKING animation frames
                            // (the original plays a taking animation
                            // during the drop) and apply the effect
                            // in one step — the observable result is
                            // the same: ammo goes down, a bonus
                            // appears.
                            //
                            // Merge gate: when the PC hasn't moved or
                            // turned since its last drop AND the
                            // previous bonus is still active AND same
                            // action AND combined quantity ≤
                            // `MAX_AMMO_PER_PILE`, the existing pile's
                            // quantity is bumped; otherwise a fresh
                            // bonus spawns. When the previous bonus is
                            // still active but the merge cap is reached
                            // (or it's a different action), the PC's
                            // facing rotates +1 sector so the next
                            // drop's "same direction" check fails and a
                            // fresh pile spawns again.
                            const MAX_AMMO_PER_PILE: u16 = 5;
                            let (action_id, amount) = match &elem.data {
                                crate::sequence::SequenceElementData::Generic { properties } => {
                                    let a = properties
                                        .get(&crate::sequence::Field::ActionId)
                                        .and_then(|v| match v {
                                            crate::sequence::FieldValue::Integer(n) => Some(*n),
                                            _ => None,
                                        });
                                    let q = properties
                                        .get(&crate::sequence::Field::Amount)
                                        .and_then(|v| match v {
                                            crate::sequence::FieldValue::Integer(n) => Some(*n),
                                            _ => None,
                                        });
                                    (a, q)
                                }
                                _ => (None, None),
                            };
                            let Some(action_id) = action_id else {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let requested = amount.unwrap_or(1) as u16;
                            let action = crate::profiles::Action::from_u32(action_id);
                            // `get_ammo` returns `u16::MAX` (0xFFFF)
                            // for actions without an ammo counter
                            // (pc_status.rs:368-386), so
                            // `!action_uses_ammo` is the equivalent
                            // sentinel test.  Treat this as terminate,
                            // not impossible.
                            if !crate::inventory::action_uses_ammo(action) {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                                continue;
                            }
                            // Refuse the drop when no walkable cell
                            // exists near the PC's hand: skip the
                            // `DROPPING_AMMO[_CROUCHED]` order and
                            // terminate.
                            if self.try_get_drop_position(owner).is_none() {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                                continue;
                            }
                            // Capture PC
                            // position/layer/sector/obstacle for the
                            // spawned bonus.
                            let pc_snap = self.get_entity(owner).map(|e| {
                                let el = e.element_data();
                                (
                                    el.position_map(),
                                    el.layer(),
                                    el.sector(),
                                    el.obstacle_index(),
                                    el.direction(),
                                    el.material(),
                                )
                            });
                            let Some((pos, layer, sector, obstacle, direction, material)) = pc_snap
                            else {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            // Decrement PC ammo, clamped to current
                            // count.
                            let status_idx = self.get_entity(owner).and_then(|e| match e {
                                crate::element::Entity::Pc(pc) => {
                                    self.pc_description_index_for_pc_data(&pc.pc)
                                }
                                _ => None,
                            });
                            let Some(status_idx) = status_idx else {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let dropped = if let Some(campaign) = self.campaign.as_mut()
                                && let Some(pc_desc) = campaign.characters.get_mut(status_idx)
                            {
                                let current = pc_desc.status.get_ammo(action);
                                let take = requested.min(current);
                                pc_desc.status.decrease_ammo(action, take);
                                take
                            } else {
                                0
                            };
                            if dropped == 0 {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            }
                            // Auto-disable the action slot when ammo
                            // reaches 0.  `dropped` was clamped to the
                            // available amount so "now empty" is
                            // detectable by re-reading.
                            let now_empty = self
                                .campaign
                                .as_ref()
                                .and_then(|c| c.characters.get(status_idx))
                                .map(|d| d.status.get_ammo(action) == 0)
                                .unwrap_or(false);
                            if now_empty {
                                self.disable_pc_action(assets, owner, action);
                            }

                            // Merge into the previously-dropped pile if
                            // PC hasn't moved/turned and the previous
                            // bonus is still alive and accepts more.
                            let prev = self.get_entity(owner).and_then(|e| match e {
                                crate::element::Entity::Pc(pc) => Some((
                                    pc.pc.last_dropped_ammo,
                                    pc.pc.last_ammo_dropping_position,
                                    pc.pc.last_dropping_direction,
                                )),
                                _ => None,
                            });
                            let same_position_and_direction = prev
                                .map(|(_, last_pos, last_dir)| {
                                    last_pos.x == pos.x
                                        && last_pos.y == pos.y
                                        && last_dir as i16 == direction
                                })
                                .unwrap_or(false);
                            // `prev_bonus_state`: Some((id, current_quantity, action))
                            // if a previous pile is still active.
                            let prev_bonus_state =
                                prev.and_then(|(last, _, _)| last).and_then(|last_id| {
                                    self.get_entity(last_id).and_then(|e| match e {
                                        crate::element::Entity::Bonus(b) if b.element.active => {
                                            Some((
                                                last_id,
                                                b.object.quantity,
                                                b.object.associated_action,
                                            ))
                                        }
                                        _ => None,
                                    })
                                });
                            let merged = if same_position_and_direction
                                && let Some((last_id, prev_qty, prev_action)) = prev_bonus_state
                                && prev_action == action
                                && prev_qty + dropped <= MAX_AMMO_PER_PILE
                            {
                                if let Some(Some(crate::element::Entity::Bonus(b))) =
                                    self.entities.get_mut(last_id.0 as usize)
                                {
                                    b.object.quantity = prev_qty + dropped;
                                }
                                tracing::debug!(
                                    pc = ?owner,
                                    ?action,
                                    dropped,
                                    bonus = ?last_id,
                                    new_qty = prev_qty + dropped,
                                    "DropAmmo: merged into previous bonus"
                                );
                                true
                            } else {
                                false
                            };

                            // When the previous bonus is still alive
                            // but we couldn't merge into it (cap reached
                            // or different action), rotate the PC by
                            // +1 sector so the next drop spawns fresh.
                            // Only fires if the PC hadn't moved/turned
                            // — otherwise the merge gate would already
                            // have rejected next time.
                            let bumped_direction = if !merged
                                && same_position_and_direction
                                && prev_bonus_state.is_some()
                            {
                                let new_dir = (direction + 1).rem_euclid(16);
                                if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                                {
                                    entity.element_data_mut().set_direction_instantly(new_dir);
                                }
                                new_dir
                            } else {
                                direction
                            };

                            let spawned_id = if !merged {
                                // Spawn a fresh bonus at the PC's
                                // position, refined via
                                // `find_authorized_position` to nudge
                                // it onto a walkable cell.
                                let spawn_pos = {
                                    let pos_geo = crate::geo2d::pt(pos.x, pos.y);
                                    let mut b = crate::geo2d::BBox2D::new();
                                    b.expand_point(pos_geo);
                                    if self
                                        .fast_grid
                                        .find_authorized_position_toward(&mut b, pos_geo, layer)
                                    {
                                        crate::element::Point2D::from(b.center())
                                    } else {
                                        pos
                                    }
                                };
                                let object_type = crate::inventory::action_to_object_type(action);
                                let mut bonus_element = crate::element::ElementData {
                                    kind: crate::element::ElementKind::ObjectBonus,
                                    active: true,
                                    // Bonus default: blipped iff this
                                    // isn't a forest level.
                                    blipped: !self.weather.is_forest_level,
                                    ..Default::default()
                                };
                                bonus_element.sprite.apply_placement(
                                    spawn_pos,
                                    layer,
                                    sector,
                                    bumped_direction,
                                    material,
                                    obstacle,
                                    crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                                        obstacle,
                                        assets.static_sight_obstacles.as_slice(),
                                    ),
                                );
                                let bonus =
                                    crate::element::Entity::Bonus(crate::element::ElementBonus {
                                        element: bonus_element,
                                        object: crate::element::ObjectData {
                                            quantity: dropped,
                                            object_type,
                                            associated_action: action,
                                            ..Default::default()
                                        },
                                    });
                                let bonus_id = self.add_entity(bonus);
                                tracing::debug!(
                                    pc = ?owner,
                                    ?action,
                                    dropped,
                                    ?bonus_id,
                                    "DropAmmo: decremented PC ammo and spawned bonus"
                                );
                                Some(bonus_id)
                            } else {
                                None
                            };

                            // Stamp the per-PC drop trackers so the
                            // next drop's merge gate evaluates against
                            // this drop.
                            if let Some(Some(crate::element::Entity::Pc(pc))) =
                                self.entities.get_mut(owner.0 as usize)
                            {
                                pc.pc.last_ammo_dropping_position = pos;
                                pc.pc.last_dropping_direction = bumped_direction as u8;
                                if let Some(new_id) = spawned_id {
                                    pc.pc.last_dropped_ammo = Some(new_id);
                                }
                            }

                            self.sequence_manager.element_terminated(seq_id, elem_idx);
                        }
                        // ── Drop ale bottle ───────────────────────
                        // Spawn a fresh ale at the PC's position,
                        // mark it detectable for all NPCs, and
                        // decrement ale ammo.  We collapse the
                        // animation into an immediate state change
                        // (no DROPPING_ALE order frames) — the
                        // observable result is the same: ammo ticks
                        // down and an ale bottle appears at the PC's
                        // feet.
                        //
                        // The Rust model represents the same dropped accessory
                        // bottle as `Entity::Bonus` + `ObjectType::Ale`.
                        // `spawn_dropped_ale` clones the `ACCESSORIES_Ale`
                        // sprite and forces `OBJECT_LYING`, so no
                        // dedicated enum variant is needed for parity.
                        Command::DropAle => {
                            let action = crate::profiles::Action::Ale;

                            let pc_snap = self.get_entity(owner).map(|e| {
                                let el = e.element_data();
                                (
                                    el.position_map(),
                                    el.layer(),
                                    el.sector(),
                                    el.obstacle_index(),
                                    el.direction(),
                                    el.material(),
                                )
                            });
                            let Some((pos, layer, sector, obstacle, direction, material)) = pc_snap
                            else {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };

                            // Decrement ammo (clamped to current count).
                            let status_idx = self.get_entity(owner).and_then(|e| match e {
                                crate::element::Entity::Pc(pc) => {
                                    self.pc_description_index_for_pc_data(&pc.pc)
                                }
                                _ => None,
                            });
                            let Some(status_idx) = status_idx else {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            let dropped = if let Some(campaign) = self.campaign.as_mut()
                                && let Some(pc_desc) = campaign.characters.get_mut(status_idx)
                            {
                                let current = pc_desc.status.get_ammo(action);
                                let take = 1u16.min(current);
                                pc_desc.status.decrease_ammo(action, take);
                                take
                            } else {
                                0
                            };
                            if dropped == 0 {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            }
                            // Auto-disable when empty.
                            let now_empty = self
                                .campaign
                                .as_ref()
                                .and_then(|c| c.characters.get(status_idx))
                                .map(|d| d.status.get_ammo(action) == 0)
                                .unwrap_or(false);
                            if now_empty {
                                self.disable_pc_action(assets, owner, action);
                            }

                            // Spawn an ale bottle at the PC's
                            // position, nudged onto a walkable cell
                            // when possible (same authorized-position
                            // handoff as generic DropAmmo above).
                            let spawn_pos = {
                                let pos_geo = crate::geo2d::pt(pos.x, pos.y);
                                let mut b = crate::geo2d::BBox2D::new();
                                b.expand_point(pos_geo);
                                if self
                                    .fast_grid
                                    .find_authorized_position_toward(&mut b, pos_geo, layer)
                                {
                                    crate::element::Point2D::from(b.center())
                                } else {
                                    pos
                                }
                            };

                            // Spawn an ale bottle at the resolved
                            // position.  We reuse the `ObjectBonus`
                            // kind because `Entity::Bonus` is the
                            // generic visible-object container; the
                            // rendering / detection payload is
                            // equivalent — `ObjectType::Ale` flags
                            // the sprite as an ale bottle (not a
                            // takable bonus).
                            let mut ale_element = crate::element::ElementData {
                                kind: crate::element::ElementKind::ObjectBonus,
                                active: true,
                                blipped: !self.weather.is_forest_level,
                                ..Default::default()
                            };
                            ale_element.sprite.apply_placement(
                                spawn_pos,
                                layer,
                                sector,
                                direction,
                                material,
                                obstacle,
                                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                                    obstacle,
                                    assets.static_sight_obstacles.as_slice(),
                                ),
                            );
                            let ale = crate::element::Entity::Bonus(crate::element::ElementBonus {
                                element: ale_element,
                                object: crate::element::ObjectData {
                                    quantity: 1,
                                    object_type: crate::element::ObjectType::Ale,
                                    associated_action: action,
                                    ..Default::default()
                                },
                            });
                            let ale_id = self.add_entity(ale);
                            tracing::debug!(
                                pc = ?owner,
                                ?ale_id,
                                "DropAle: decremented PC ale ammo and spawned ale bottle"
                            );
                            self.sequence_manager.element_terminated(seq_id, elem_idx);
                        }
                        Command::ThrowNet => {
                            let target_pos = match &elem.data {
                                crate::sequence::SequenceElementData::Generic { properties } => {
                                    match properties.get(&crate::sequence::Field::NetTarget) {
                                        Some(crate::sequence::FieldValue::Point2D { x, y }) => {
                                            Some(crate::element::Point2D { x: *x, y: *y })
                                        }
                                        Some(crate::sequence::FieldValue::Point3D {
                                            x, y, ..
                                        }) => Some(crate::element::Point2D { x: *x, y: *y }),
                                        _ => None,
                                    }
                                }
                                _ => None,
                            };
                            match target_pos {
                                Some(pos) => {
                                    if !self.has_ammo(owner, crate::profiles::Action::Net) {
                                        self.sequence_manager.element_impossible(seq_id, elem_idx);
                                    } else {
                                        match abilities::begin_throw_net(
                                            &mut self.entities,
                                            &mut self.sequence_manager,
                                            owner,
                                            pos,
                                            seq_id,
                                            elem_idx,
                                            &mut self.next_order_id,
                                        ) {
                                            AbilityBeginResult::Started => {
                                                self.sequence_manager
                                                    .element_in_progress(seq_id, elem_idx);
                                            }
                                            AbilityBeginResult::Impossible => {
                                                self.sequence_manager
                                                    .element_impossible(seq_id, elem_idx);
                                            }
                                        }
                                    }
                                }
                                None => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::ThrowPurse => {
                            let target_pos = match &elem.data {
                                crate::sequence::SequenceElementData::Generic { properties } => {
                                    match properties.get(&crate::sequence::Field::PurseTarget) {
                                        Some(crate::sequence::FieldValue::Point2D { x, y }) => {
                                            Some(crate::element::Point2D { x: *x, y: *y })
                                        }
                                        Some(crate::sequence::FieldValue::Point3D {
                                            x, y, ..
                                        }) => Some(crate::element::Point2D { x: *x, y: *y }),
                                        _ => None,
                                    }
                                }
                                _ => None,
                            };
                            match target_pos {
                                Some(pos) => {
                                    if !self.has_ammo(owner, crate::profiles::Action::Purse) {
                                        self.sequence_manager.element_impossible(seq_id, elem_idx);
                                    } else {
                                        match abilities::begin_throw_purse(
                                            &mut self.entities,
                                            &mut self.sequence_manager,
                                            owner,
                                            pos,
                                            seq_id,
                                            elem_idx,
                                            &mut self.next_order_id,
                                        ) {
                                            AbilityBeginResult::Started => {
                                                self.sequence_manager
                                                    .element_in_progress(seq_id, elem_idx);
                                            }
                                            AbilityBeginResult::Impossible => {
                                                self.sequence_manager
                                                    .element_impossible(seq_id, elem_idx);
                                            }
                                        }
                                    }
                                }
                                None => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }
                        Command::ThrowWaspNest => {
                            let target_pos = match &elem.data {
                                crate::sequence::SequenceElementData::Generic { properties } => {
                                    match properties.get(&crate::sequence::Field::WaspNestTarget) {
                                        Some(crate::sequence::FieldValue::Point2D { x, y }) => {
                                            Some(crate::element::Point2D { x: *x, y: *y })
                                        }
                                        Some(crate::sequence::FieldValue::Point3D {
                                            x, y, ..
                                        }) => Some(crate::element::Point2D { x: *x, y: *y }),
                                        _ => None,
                                    }
                                }
                                _ => None,
                            };
                            match target_pos {
                                Some(pos) => {
                                    if !self.has_ammo(owner, crate::profiles::Action::WaspNest) {
                                        self.sequence_manager.element_impossible(seq_id, elem_idx);
                                    } else {
                                        match abilities::begin_throw_wasp_nest(
                                            &mut self.entities,
                                            &mut self.sequence_manager,
                                            owner,
                                            pos,
                                            seq_id,
                                            elem_idx,
                                            &mut self.next_order_id,
                                        ) {
                                            AbilityBeginResult::Started => {
                                                self.sequence_manager
                                                    .element_in_progress(seq_id, elem_idx);
                                            }
                                            AbilityBeginResult::Impossible => {
                                                self.sequence_manager
                                                    .element_impossible(seq_id, elem_idx);
                                            }
                                        }
                                    }
                                }
                                None => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }

                        // ── ThrowApple / ThrowStone ──────────────
                        // When the THROWING_APPLE / THROWING_STONE
                        // animation first plays, begin the ability;
                        // on completion, the engine spawns the
                        // projectile.
                        Command::ThrowApple | Command::ThrowStone => {
                            let (target_opt, action) = match cmd {
                                Command::ThrowApple => (
                                    match &elem.data {
                                        crate::sequence::SequenceElementData::Interaction {
                                            antagonist,
                                        } => *antagonist,
                                        _ => None,
                                    },
                                    crate::profiles::Action::Apple,
                                ),
                                Command::ThrowStone => (
                                    match &elem.data {
                                        crate::sequence::SequenceElementData::Interaction {
                                            antagonist,
                                        } => *antagonist,
                                        _ => None,
                                    },
                                    crate::profiles::Action::Stone,
                                ),
                                _ => unreachable!(),
                            };
                            let target = match target_opt {
                                Some(t) => t,
                                None => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                    continue;
                                }
                            };
                            if !self.has_ammo(owner, action) {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            }
                            let begin = match cmd {
                                Command::ThrowApple => abilities::begin_throw_apple(
                                    &mut self.entities,
                                    &mut self.sequence_manager,
                                    owner,
                                    target,
                                    seq_id,
                                    elem_idx,
                                    &mut self.next_order_id,
                                ),
                                Command::ThrowStone => abilities::begin_throw_stone(
                                    &mut self.entities,
                                    &mut self.sequence_manager,
                                    owner,
                                    target,
                                    seq_id,
                                    elem_idx,
                                    &mut self.next_order_id,
                                ),
                                _ => unreachable!(),
                            };
                            match begin {
                                AbilityBeginResult::Started => {
                                    self.sequence_manager.element_in_progress(seq_id, elem_idx);
                                }
                                AbilityBeginResult::Impossible => {
                                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                                }
                            }
                        }

                        // ── Turn ───────────────────────────────
                        // Rotate the actor to face the `CameraPoint`
                        // property (or `Direction` property if no
                        // point), then push a single `Turning` order.
                        // The element terminates when the animation's
                        // sprite reports completion.  TURN and
                        // TURN_FAST share an identical body — both
                        // read CameraPoint / Direction from the
                        // element and push Turning onto the order
                        // queue; only Upright posture is legal.
                        Command::Turn | Command::TurnFast => {
                            let elem_props = self.sequence_manager.get_element(seq_id, elem_idx);
                            let camera_point = elem_props
                                .and_then(|e| e.get_property(crate::sequence::Field::CameraPoint))
                                .and_then(|v| match v {
                                    crate::sequence::FieldValue::Point2D { x, y } => Some((*x, *y)),
                                    _ => None,
                                });
                            let explicit_direction = elem_props
                                .and_then(|e| e.get_property(crate::sequence::Field::Direction))
                                .and_then(|v| match v {
                                    crate::sequence::FieldValue::Integer(d) => Some(*d as i16),
                                    _ => None,
                                });
                            if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize) {
                                // Apply the direction: explicit wins;
                                // otherwise face the camera point.
                                // Use `set_direction_goal` (not
                                // `set_direction_instantly`) so the
                                // body rotates progressively via
                                // `turn_fast()` in the Turning
                                // order's Execute loop.  Snapping
                                // `direction == direction_goal` would
                                // make `turn_fast()` short-circuit on
                                // the first tick.
                                if let Some(dir) = explicit_direction {
                                    entity.element_data_mut().set_direction_goal(dir);
                                } else if let Some((tx, ty)) = camera_point {
                                    let pos = entity.element_data().position_map();
                                    let dx = tx - pos.x;
                                    let dy = ty - pos.y;
                                    // Convert `(camera_point -
                                    // position_map)` into the 0..15
                                    // facing sector.
                                    let dir =
                                        crate::position_interface::vector_to_sector_0_to_15_iso(
                                            dx, dy,
                                        );
                                    entity.element_data_mut().set_direction_goal(dir);
                                }
                            }
                            // Push the Turning animation onto the Turn
                            // element.  The animation driver reads the
                            // front order via `current_order_for_actor`
                            // and the default `AdvanceElement` completion
                            // terminates the element when the rotation
                            // finishes (see Turning-specific `turn_fast`
                            // gate in `tick_entity_animations`).
                            self.push_new_order(
                                seq_id,
                                elem_idx,
                                crate::order::OrderType::Turning,
                                0.0,
                                0.0,
                            );
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }

                        // Face the element's antagonist, then push
                        // Turning.  Carried by
                        // `SequenceElementData::Interaction`.
                        Command::TurnElement => {
                            let antagonist = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            if let Some(antag_id) = antagonist {
                                let antag_pos = self
                                    .get_entity(antag_id)
                                    .map(|e| e.element_data().position_map());
                                if let (Some(antag_pos), Some(Some(entity))) =
                                    (antag_pos, self.entities.get_mut(owner.0 as usize))
                                {
                                    let pos = entity.element_data().position_map();
                                    let dir =
                                        crate::position_interface::vector_to_sector_0_to_15_iso(
                                            antag_pos.x - pos.x,
                                            antag_pos.y - pos.y,
                                        );
                                    entity.element_data_mut().set_direction_instantly(dir);
                                }
                            }
                            self.push_new_order(
                                seq_id,
                                elem_idx,
                                crate::order::OrderType::Turning,
                                0.0,
                                0.0,
                            );
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }

                        // Owner-ful Freeze pushes a `Freezing` order
                        // onto the element.  The engine-side
                        // `ExecuteImmediateEngine` arm at the bottom
                        // of this file handles non-owner Freeze
                        // (which collapses into FreezeAll).
                        Command::Freeze => {
                            self.push_new_order(
                                seq_id,
                                elem_idx,
                                crate::order::OrderType::Freezing,
                                0.0,
                                0.0,
                            );
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }

                        // ── Point / GatherSoldiers ─────────────
                        // Each pushes a single one-shot animation
                        // order (`Pointing` / `GatheringSoldiers`)
                        // with `compute_direction = false`.  Point
                        // reads `Direction` and sets the actor's
                        // facing before the anim; GatherSoldiers has
                        // no direction.  Both terminate the sequence
                        // element on animation completion, wired via
                        // `AiAnimCompletion::SequenceElement`.
                        Command::Point | Command::GatherSoldiers => {
                            let order_type = match elem.command {
                                Command::Point => crate::order::OrderType::Pointing,
                                Command::GatherSoldiers => {
                                    crate::order::OrderType::GatheringSoldiers
                                }
                                _ => unreachable!(),
                            };
                            let explicit_direction = if elem.command == Command::Point {
                                self.sequence_manager
                                    .get_element(seq_id, elem_idx)
                                    .and_then(|e| e.get_property(crate::sequence::Field::Direction))
                                    .and_then(|v| match v {
                                        crate::sequence::FieldValue::Integer(d) => Some(*d as i16),
                                        _ => None,
                                    })
                            } else {
                                None
                            };
                            if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                                && let Some(dir) = explicit_direction
                            {
                                entity.element_data_mut().set_direction_instantly(dir);
                            }
                            let mut order = crate::order::Order::new(
                                order_type,
                                0.0,
                                0.0,
                                self.alloc_order_id(),
                            );
                            order.compute_direction = false;
                            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }

                        // ── Wait (soldier-specific override) ───
                        //   - attentive + upright + waiting + alive →
                        //     WAITING_ALERTED
                        //   - leaning out with AimingWithBow{,Down} →
                        //     AIMING_WITH_BOW_LEANING_OUT
                        //   - leaning out otherwise → LEANING_OUT
                        //   - anything else → fall through to NPC
                        //     base (not dispatched here — terminates,
                        //     which matches the existing catch-all).
                        // WAIT_TIMER additionally records `wait_time`
                        // from the element's Timer property.
                        // WAIT_FREE_LIFT is kept on its own path
                        // above for the lift-occupancy bookkeeping;
                        // we don't intercept it here.
                        Command::Wait | Command::WaitTimer => {
                            let (
                                is_soldier,
                                is_pc,
                                posture,
                                action_state,
                                is_attentive,
                                is_dead,
                                is_unconscious,
                                is_swordfighting,
                                is_stuck_under_net,
                                carrier_is_vip,
                            ) = {
                                let ent = self.get_entity(owner);
                                let is_soldier = ent.map(|e| e.is_soldier()).unwrap_or(false);
                                let is_pc = ent.map(|e| e.is_pc()).unwrap_or(false);
                                let posture =
                                    ent.map(|e| e.element_data().posture).unwrap_or_default();
                                let action_state = ent
                                    .and_then(|e| e.actor_data())
                                    .map(|a| a.action_state)
                                    .unwrap_or_default();
                                let attentive =
                                    ent.and_then(|e| e.enemy_ai()).is_some_and(|e| e.attentive);
                                let dead = ent.is_some_and(|e| e.is_dead());
                                let unc = ent
                                    .and_then(|e| e.human_data())
                                    .is_some_and(|h| h.unconscious);
                                // Swordfighting iff the human's
                                // opponent list is non-empty.
                                let sword = ent
                                    .and_then(|e| e.human_data())
                                    .is_some_and(|h| !h.opponents.is_empty());
                                // Stuck-under-net iff the counter
                                // is positive.
                                let stuck = ent
                                    .and_then(|e| e.human_data())
                                    .is_some_and(|h| h.stuck_under_nets_counter > 0);
                                // Carrier-is-VIP — only meaningful
                                // for the CARRIED branch below.
                                let carrier_id =
                                    ent.and_then(|e| e.human_data()).and_then(|h| h.carrier);
                                let carrier_vip = carrier_id
                                    .and_then(|cid| self.get_entity(cid))
                                    .is_some_and(|c| self.is_entity_vip(assets, c));
                                (
                                    is_soldier,
                                    is_pc,
                                    posture,
                                    action_state,
                                    attentive,
                                    dead,
                                    unc,
                                    sword,
                                    stuck,
                                    carrier_vip,
                                )
                            };

                            // Pick the starting order for the wait
                            // element.  Soldier overrides handle the
                            // attentive + leaning arms; the
                            // posture-based fallback covers everyone
                            // else.
                            let after_state = self
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .map(|e| e.action_state_after_transition)
                                .unwrap_or_default();
                            // PC-specific WAIT posture arms.  PCs in
                            // HelpingToClimb / CarryingOnShoulders /
                            // OnShoulders / CarryingCorpse /
                            // SimulatingBeggar / Spy /
                            // AnonymousArcher / Tree posture, or
                            // Upright + Listening, queue a posture-
                            // specific idle animation rather than
                            // falling through to the base human
                            // matrix.
                            let pc_posture_anim = if is_pc {
                                use crate::element::{ActionState as AS, Posture as P};
                                use crate::order::OrderType as OT;
                                match posture {
                                    P::HelpingToClimb => Some(OT::WaitingHelpingClimbing),
                                    P::CarryingOnShoulders => Some(OT::WaitingCarryingOnShoulders),
                                    P::OnShoulders => Some(OT::WaitingOnShoulders),
                                    P::CarryingCorpse => Some(OT::WaitingWithCorpse),
                                    P::SimulatingBeggar => Some(OT::SimulatingBeggar),
                                    P::Spy => Some(OT::WaitingCape),
                                    P::AnonymousArcher => Some(match after_state {
                                        AS::AimingWithBow => OT::AimingWithBowAnonymous,
                                        AS::AimingWithBowUp => OT::AimingWithBowUpAnonymous,
                                        _ => OT::WaitingCapeAnonymousArcher,
                                    }),
                                    P::Tree => Some(OT::WaitingHidden),
                                    // Upright + LISTENING queues
                                    // LISTENING and arms `wait_time`
                                    // (handled below).  Otherwise
                                    // fall through.
                                    P::Upright if action_state == AS::Listening => {
                                        Some(OT::Listening)
                                    }
                                    _ => None,
                                }
                            } else {
                                None
                            };
                            // Track the conscious-Lying-stuck-under-
                            // net side-effect:
                            // `SetPosture(StuckUnderNet)` runs before
                            // the order is queued.
                            let mut set_posture_stuck_under_net = false;
                            let anim = if let Some(pc_anim) = pc_posture_anim {
                                Some(pc_anim)
                            } else if is_soldier
                                && is_attentive
                                && posture == crate::element::Posture::Upright
                                && action_state == crate::element::ActionState::Waiting
                                && !is_dead
                                && !is_unconscious
                            {
                                Some(crate::order::OrderType::WaitingAlerted)
                            } else if is_soldier && posture == crate::element::Posture::LeaningOut {
                                Some(match after_state {
                                    crate::element::ActionState::AimingWithBow
                                    | crate::element::ActionState::AimingWithBowDown => {
                                        crate::order::OrderType::AimingWithBowLeaningOut
                                    }
                                    _ => crate::order::OrderType::LeaningOut,
                                })
                            } else {
                                use crate::element::{ActionState as AS, Posture as P};
                                use crate::order::OrderType as OT;
                                // WAIT/WAIT_TIMER posture matrix.
                                // The matrix keys off the element's
                                // action-state-after-transition for
                                // the stance arms.  The Upright
                                // IsSwordfighting branch routes a
                                // non-sword stance through
                                // TransitionRaisingSword so the actor
                                // re-enters combat stance before
                                // idling.
                                let upright_anim = if is_swordfighting {
                                    match after_state {
                                        AS::ParryingSword | AS::ParryingSwordLow => {
                                            OT::ParryingSword
                                        }
                                        AS::WaitingSword
                                        | AS::MovingSword
                                        | AS::MovingFastSword => OT::WaitingSword,
                                        _ => OT::TransitionRaisingSword,
                                    }
                                } else {
                                    match after_state {
                                        AS::HoldingShield
                                        | AS::ParryingShield
                                        | AS::MovingShield => OT::WaitingShield,
                                        AS::AimingWithBow => OT::AimingWithBow,
                                        AS::AimingWithBowUp => OT::AimingWithBowUp,
                                        AS::WaitingSword
                                        | AS::MovingSword
                                        | AS::MovingFastSword => OT::WaitingSword,
                                        AS::Menacing => OT::Menacing,
                                        AS::Sleeping => OT::SleepingUpright,
                                        AS::ParryingSword | AS::ParryingSwordLow => {
                                            OT::ParryingSword
                                        }
                                        // Default falls through to
                                        // the base, which queues
                                        // WAITING_UPRIGHT_BORED for
                                        // Upright posture.
                                        _ => OT::WaitingUprightBored,
                                    }
                                };
                                match posture {
                                    P::Upright => Some(upright_anim),
                                    P::Crouched => Some(OT::WaitingCrouched),
                                    P::OnWall | P::OnLadder => Some(OT::Freezing),
                                    P::Sitting => Some(OT::Sitting),
                                    // Unconscious actors (or any
                                    // WAIT_TIMER) play the
                                    // BeingUnconscious idle loop; the
                                    // stance suffix tracks what they
                                    // were holding when they
                                    // collapsed.
                                    P::Lying
                                        if is_unconscious || elem.command == Command::WaitTimer =>
                                    {
                                        Some(match after_state {
                                            s if s.is_sword() || s == AS::Menacing => {
                                                OT::BeingUnconsciousSword
                                            }
                                            s if s.is_bow() => OT::BeingUnconsciousBow,
                                            _ => OT::BeingUnconscious,
                                        })
                                    }
                                    // Conscious Lying + plain WAIT.
                                    // If the actor is stuck under a
                                    // net, snap the posture to
                                    // StuckUnderNet and queue the
                                    // lying-net pose; otherwise stand
                                    // back up with the stance-
                                    // appropriate STANDING_UP variant.
                                    P::Lying => {
                                        if is_stuck_under_net {
                                            set_posture_stuck_under_net = true;
                                            Some(OT::LyingStuckUnderNet)
                                        } else {
                                            Some(match after_state {
                                                s if s.is_sword() || s == AS::Menacing => {
                                                    OT::StandingUpSword
                                                }
                                                s if s.is_bow() => OT::StandingUpBow,
                                                _ => OT::StandingUp,
                                            })
                                        }
                                    }
                                    P::DeadBack => Some(match after_state {
                                        AS::WaitingSword | AS::Menacing => {
                                            OT::BeingDeadFallenBackSword
                                        }
                                        AS::AimingWithBow | AS::AimingWithBowDown => {
                                            OT::BeingDeadFallenBackBow
                                        }
                                        _ => OT::BeingDeadFallenBack,
                                    }),
                                    P::Dead => Some(match after_state {
                                        AS::WaitingSword => OT::BeingDeadSword,
                                        AS::AimingWithBow | AS::AimingWithBowDown => {
                                            OT::BeingDeadBow
                                        }
                                        _ => OT::BeingDead,
                                    }),
                                    // CARRIED is asserted
                                    // unreachable upstream, but the
                                    // matrix below still selects a
                                    // stance.  Apply the matrix and
                                    // log a warning if it fires (we
                                    // don't crash the game).
                                    P::Carried => {
                                        tracing::warn!(
                                            ?owner,
                                            "Wait/Translate: CARRIED posture reached \
                                             (asserted unreachable upstream); \
                                             queuing BeingCarried{{LittleJohn|PeasantC}}"
                                        );
                                        Some(if carrier_is_vip {
                                            OT::BeingCarriedLittleJohn
                                        } else {
                                            OT::BeingCarriedPeasantC
                                        })
                                    }
                                    P::Tied => Some(OT::BeingTied),
                                    // `Special` is the leisure idle
                                    // pose.
                                    P::Leisure => Some(OT::Special),
                                    P::StuckUnderNet => Some(OT::LyingStuckUnderNet),
                                    // Spy/Tree/Beggar/HelpingToClimb/
                                    // CarryingOnShoulders/OnShoulders/
                                    // CarryingCorpse/AnonymousArcher
                                    // are PC-specific and handled by
                                    // `pc_posture_anim` above.  The
                                    // base human matrix has no arm
                                    // for them.
                                    _ => None,
                                }
                            };

                            // `WAIT_TIMER`: record the timer value
                            // on the actor so later tick code can
                            // decrement it.
                            if elem.command == Command::WaitTimer {
                                let timer_val = self
                                    .sequence_manager
                                    .get_element(seq_id, elem_idx)
                                    .and_then(|e| e.get_property(crate::sequence::Field::Timer))
                                    .and_then(|v| match v {
                                        crate::sequence::FieldValue::Integer(n) => Some(*n),
                                        _ => None,
                                    })
                                    .unwrap_or(0);
                                if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                                    && let Some(actor) = entity.actor_data_mut()
                                {
                                    actor.wait_time = timer_val;
                                }
                            }
                            // Upright + LISTENING forces
                            // `wait_time = TIME_LISTEN_WAIT` (25
                            // frames) even for plain WAIT (not
                            // WAIT_TIMER).
                            if is_pc
                                && posture == crate::element::Posture::Upright
                                && action_state == crate::element::ActionState::Listening
                            {
                                const TIME_LISTEN_WAIT: u32 = 25;
                                if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                                    && let Some(actor) = entity.actor_data_mut()
                                {
                                    actor.wait_time = TIME_LISTEN_WAIT;
                                }
                            }

                            // `SetPosture(StuckUnderNet)` happens
                            // inline inside Translate, before the
                            // order is queued, when a conscious Lying
                            // actor is stuck under a net.
                            if set_posture_stuck_under_net
                                && let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                            {
                                entity
                                    .element_data_mut()
                                    .set_posture(crate::element::Posture::StuckUnderNet);
                            }

                            if let Some(anim_ot) = anim {
                                let mut order = crate::order::Order::new(
                                    anim_ot,
                                    0.0,
                                    0.0,
                                    self.alloc_order_id(),
                                );
                                order.compute_direction = false;
                                // Per-arm completion semantics for
                                // BORED / BORED_RANDOM (advance only
                                // on TERMINATED, with 1/10 roll +
                                // NewID in place) live in
                                // `dispatch_arm_completion`
                                // (engine/animation.rs) — no order-
                                // level flag required here.
                                self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                                self.sequence_manager.element_in_progress(seq_id, elem_idx);
                            } else {
                                // No starting order — nothing visible to
                                // drive.  Terminate so the element
                                // doesn't sit idle.
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                            }
                        }

                        // ── NPC-specific one-shot anims ────────
                        // Each command appends one animation order
                        // with `compute_direction = false`, so we
                        // book it through `active_ai_anim` and bind
                        // sequence termination to its DONE — matching
                        // the existing `Point` arm above.  Posture
                        // flips (Upright→Sitting / Upright→Leisure)
                        // are handled by the animation-completion
                        // side effects in `animation.rs`.
                        //
                        // `launch_element_for_owner`/single-order launch
                        // call `generate_transition` before this command
                        // body is reached.  For these NPC commands the
                        // transition flags match legacy behavior, so any
                        // needed leave-action/posture orders have already been
                        // queued ahead of the command's own animation.
                        Command::SitDown | Command::BeggarShowFace | Command::EnterLeisure => {
                            let order_type = match elem.command {
                                Command::SitDown => {
                                    crate::order::OrderType::TransitionWaitingUprightSitting
                                }
                                Command::BeggarShowFace => {
                                    crate::order::OrderType::BeggarShowingFace
                                }
                                Command::EnterLeisure => {
                                    crate::order::OrderType::TransitionWaitingUprightSpecial
                                }
                                _ => unreachable!(),
                            };
                            // Build the order with
                            // `compute_direction = false` for
                            // SIT_DOWN / BEGGAR_SHOW_FACE /
                            // ENTER_LEISURE.  In-place anims never
                            // invoke `compute_increment_all`, so the
                            // flag is dead today, but keeping it
                            // honest leaves the contract intact if a
                            // future order-type wires movement.
                            let id = self.alloc_order_id();
                            let mut order = crate::order::Order::new(order_type, 0.0, 0.0, id);
                            order.compute_direction = false;
                            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }

                        // ── Menace / Sleep transitions ─────────
                        // Each pushes a fixed sequence of transition
                        // orders with `compute_direction = false`.
                        // The animation system's DONE/TERMINATED
                        // hooks in `animation.rs` flip posture /
                        // action_state appropriately when each order
                        // finishes, so the sequence element itself
                        // can terminate immediately — the visual
                        // transition plays off the actor's order
                        // queue.
                        Command::StartMenace
                        | Command::StopMenace
                        | Command::StopSleep
                        | Command::LowerBowLeanOut
                        | Command::RaiseBowLeanOut => {
                            let command = elem.command;
                            // Push `compute_direction = false`
                            // transition orders onto the owning
                            // sequence element — these are one- and
                            // two-order transition arms.
                            let push = |engine: &mut EngineInner, ot: crate::order::OrderType| {
                                let id = engine.alloc_order_id();
                                let mut order = crate::order::Order::new(ot, 0.0, 0.0, id);
                                order.compute_direction = false;
                                engine
                                    .sequence_manager
                                    .push_order_on(seq_id, elem_idx, order);
                            };
                            match command {
                                Command::StartMenace => {
                                    push(self, crate::order::OrderType::TransitionRaisingSword);
                                    push(
                                        self,
                                        crate::order::OrderType::TransitionWaitingSwordMenacing,
                                    );
                                }
                                Command::StopMenace => {
                                    push(
                                        self,
                                        crate::order::OrderType::TransitionMenacingWaitingSword,
                                    );
                                    push(self, crate::order::OrderType::TransitionLoweringSword);
                                }
                                Command::StopSleep => {
                                    push(
                                        self,
                                        crate::order::OrderType::TransitionSleepingWaitingUpright,
                                    );
                                }
                                // Single transition order per
                                // command.
                                Command::LowerBowLeanOut => {
                                    push(
                                        self,
                                        crate::order::OrderType::TransitionLoweringBowLeaningOut,
                                    );
                                }
                                Command::RaiseBowLeanOut => {
                                    push(
                                        self,
                                        crate::order::OrderType::TransitionRaisingBowLeaningOut,
                                    );
                                }
                                _ => unreachable!(),
                            }
                            if matches!(
                                command,
                                Command::LowerBowLeanOut | Command::RaiseBowLeanOut
                            ) {
                                self.sequence_manager.element_in_progress(seq_id, elem_idx);
                            } else {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                            }
                        }

                        // ── DrinkAle / Take ────────────────────
                        // DrinkAle / Take push a single interaction
                        // order whose animation (DRINKING_ALE /
                        // TAKING) references the antagonist (bottle /
                        // purse / coin).  The corresponding Execute
                        // handlers hide / remove the antagonist on
                        // DONE and bump money / blood-alcohol on
                        // TERMINATED.  Book through `active_ai_anim`
                        // with the antagonist threaded along so the
                        // `apply_soldier_execute_side_effects`
                        // handler picks up the target.
                        Command::DrinkAle | Command::Take => {
                            let command = elem.command;
                            let owner_is_pc = self.get_entity(owner).is_some_and(|e| e.is_pc());
                            let antagonist = self
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .and_then(|e| match &e.data {
                                    crate::sequence::SequenceElementData::Interaction {
                                        antagonist,
                                    } => *antagonist,
                                    _ => None,
                                });

                            // Validate antagonist matches
                            // expectations — the original asserts on
                            // object type.  Panicking here rather
                            // than silently accepting any entity
                            // lets bad scripts / AI decisions
                            // fail loudly instead of drinking invisible
                            // purses.
                            if let Some(a_id) = antagonist {
                                let ant = self.get_entity(a_id);
                                // Scroll/Bonus/Projectile/Net all share
                                // ObjectData — use the shared accessor so
                                // PCs picking up scrolls aren't rejected.
                                let obj_type =
                                    ant.and_then(|e| e.object_data().map(|o| o.object_type));
                                match command {
                                    Command::DrinkAle => {
                                        assert!(
                                            matches!(
                                                obj_type,
                                                Some(crate::element::ObjectType::Ale)
                                            ),
                                            "DrinkAle: antagonist {:?} has object_type {:?}; expected Ale",
                                            a_id,
                                            obj_type
                                        );
                                    }
                                    // Soldiers restrict TAKE to
                                    // Net / Purse / Coin.  PCs accept
                                    // any object antagonist (default
                                    // TAKING animation, Net gets
                                    // TAKING_NET).  Scrolls and
                                    // bonuses reach here via PC
                                    // pickup paths.
                                    Command::Take if !owner_is_pc => {
                                        assert!(
                                            matches!(
                                                obj_type,
                                                Some(
                                                    crate::element::ObjectType::Net
                                                        | crate::element::ObjectType::Purse
                                                        | crate::element::ObjectType::Coin
                                                )
                                            ),
                                            "Take (soldier): antagonist {:?} has object_type {:?}; expected Net/Purse/Coin",
                                            a_id,
                                            obj_type
                                        );
                                    }
                                    Command::Take => {
                                        assert!(
                                            obj_type.is_some(),
                                            "Take (PC): antagonist {:?} is not an object",
                                            a_id
                                        );
                                    }
                                    _ => {}
                                }
                            }

                            if matches!(command, Command::DrinkAle)
                                || matches!(command, Command::Take) && !owner_is_pc
                            {
                                let a_id = antagonist.unwrap_or_else(|| {
                                    panic!("{:?}: missing interaction antagonist", command)
                                });
                                let direction_goal = {
                                    let owner_pos = self.entities[owner.0 as usize]
                                        .as_ref()
                                        .unwrap_or_else(|| {
                                            panic!("{:?}: owner {:?} is missing", command, owner)
                                        })
                                        .element_data()
                                        .position_map();
                                    let antagonist_pos = self.entities[a_id.0 as usize]
                                        .as_ref()
                                        .unwrap_or_else(|| {
                                            panic!(
                                                "{:?}: antagonist {:?} is missing",
                                                command, a_id
                                            )
                                        })
                                        .element_data()
                                        .position_map();
                                    crate::position_interface::vector_to_sector_0_to_15_iso(
                                        antagonist_pos.x - owner_pos.x,
                                        antagonist_pos.y - owner_pos.y,
                                    )
                                };
                                self.entities[owner.0 as usize]
                                    .as_mut()
                                    .unwrap_or_else(|| {
                                        panic!("{:?}: owner {:?} is missing", command, owner)
                                    })
                                    .element_data_mut()
                                    .set_direction_goal(direction_goal);
                            }

                            // PCs picking up a net play
                            // `TakingNet` rather than the generic
                            // `Taking`.
                            let antagonist_is_net = antagonist
                                .and_then(|a| self.get_entity(a))
                                .is_some_and(|e| matches!(e, crate::element::Entity::Net(_)));
                            let order_type = match command {
                                Command::DrinkAle => crate::order::OrderType::DrinkingAle,
                                Command::Take if antagonist_is_net => {
                                    crate::order::OrderType::TakingNet
                                }
                                Command::Take => crate::order::OrderType::Taking,
                                _ => unreachable!(),
                            };
                            let mut order = crate::order::Order::new(
                                order_type,
                                0.0,
                                0.0,
                                self.alloc_order_id(),
                            );
                            if let Some(a) = antagonist {
                                order = order.with_antagonist(a);
                            }
                            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }

                        // ── UnlockDoor ─────────────────────────
                        // The PC pushes a single `UnlockingDoor`
                        // order (or `UnlockingTrap` when the door is
                        // a building-trap) and the door's `locked_pc`
                        // flag flips off when the lockpick animation
                        // finishes.  We book the anim via
                        // `active_ai_anim` + `UnlockDoor` completion
                        // so the flag flip + element termination
                        // happen on animation end.  Target door is
                        // read from the `Field::Door` property set
                        // by `build_gate_movement_sequence`.
                        Command::UnlockDoor => {
                            let door_id = self
                                .sequence_manager
                                .get_element(seq_id, elem_idx)
                                .and_then(|e| e.get_property(crate::sequence::Field::Door))
                                .and_then(|v| match v {
                                    crate::sequence::FieldValue::DoorId(id) => Some(*id),
                                    crate::sequence::FieldValue::Integer(id) => {
                                        Some(crate::gate::DoorIndex(*id))
                                    }
                                    _ => None,
                                });
                            let Some(id) = door_id else {
                                // No target door — can't proceed; just
                                // terminate so the sequence doesn't stall.
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                                continue;
                            };
                            // Pick UnlockingDoor vs UnlockingTrap
                            // by door type.
                            let anim_type = self
                                .mission_script
                                .as_ref()
                                .and_then(|s| s.game_host())
                                .and_then(|h| h.doors.get(usize::from(id)))
                                .map(|d| match d.door_type {
                                    crate::gate::DoorType::BuildingTrap => {
                                        crate::order::OrderType::UnlockingTrap
                                    }
                                    _ => crate::order::OrderType::UnlockingDoor,
                                })
                                .unwrap_or(crate::order::OrderType::UnlockingDoor);
                            tracing::debug!(
                                door_id = %id,
                                entity = ?owner,
                                ?anim_type,
                                "UnlockDoor: starting lockpick animation"
                            );
                            let order = crate::order::Order::new(
                                anim_type,
                                0.0,
                                0.0,
                                self.alloc_order_id(),
                            )
                            .with_completion(
                                crate::order::OrderCompletion::UnlockDoor { door_id: id },
                            );
                            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }

                        // ── Jump ────────────────────────────────
                        // Build a step list covering the run-up,
                        // airborne trajectory, and landing
                        // transitions, then drive the actor through
                        // them via `tick_active_jumps`.  If the jump
                        // can't be installed (missing data) the
                        // element is terminated so the sequence
                        // doesn't stall.
                        Command::Jump => {
                            if self.start_jump(assets, owner, seq_id, elem_idx) {
                                self.sequence_manager.element_in_progress(seq_id, elem_idx);
                            } else {
                                tracing::warn!(
                                    entity = ?owner,
                                    seq = ?seq_id,
                                    elem = elem_idx,
                                    "Jump: failed to install ActiveJump — terminating element"
                                );
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                            }
                        }

                        Command::ActivateApple
                        | Command::ActivateArrow
                        | Command::ActivateHandle
                        | Command::ActivateHeal
                        | Command::ActivateLever
                        | Command::ActivateMoney
                        | Command::ActivateSearch
                        | Command::ActivateStone
                        | Command::ActivateSword => {
                            // The target dispatches each `Activate*`
                            // to its own
                            // `IElementTargetScript::ActivatedBy*`.
                            //
                            // The antagonist carried on the sequence
                            // element is the PC who initiated the
                            // action.  We collect the call here and
                            // dispatch after the action loop so the
                            // script can safely borrow
                            // `self.entities`.
                            let method = match cmd {
                                Command::ActivateApple => "ActivatedByApple",
                                Command::ActivateArrow => "ActivatedByArrow",
                                Command::ActivateHandle => "ActivatedByHand",
                                Command::ActivateHeal => "ActivatedByHeal",
                                Command::ActivateLever => "ActivatedByLever",
                                Command::ActivateMoney => "ActivatedByMoney",
                                Command::ActivateSearch => "ActivatedBySearch",
                                Command::ActivateStone => "ActivatedByStone",
                                Command::ActivateSword => "ActivatedBySword",
                                _ => unreachable!(),
                            };
                            let antagonist = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            // Owner must be an FX target — the
                            // launch sites assert it and the
                            // `Activate*` dispatch is only valid for
                            // FX targets.  A malformed sequence
                            // panics — match that here.
                            debug_assert!(
                                self.get_entity(owner)
                                    .is_some_and(|e| e.kind().is_fx_target()),
                                "{method} dispatched on non-FX-target owner {owner:?}",
                            );
                            let target_handle = crate::natives::GameHost::actor_handle(owner);
                            let pc_handle = antagonist
                                .map(crate::natives::GameHost::actor_handle)
                                .unwrap_or(0);
                            pending_target_activations.push((target_handle, pc_handle, method));
                            self.sequence_manager.element_terminated(seq_id, elem_idx);
                        }

                        // Script-recorded PlayAnim / PlayAnimLoop /
                        // PlayAnimFreeze / PlayAnimFrozen.  C++ translates these to
                        // PLAY_CUSTOM non-animations for actors, which
                        // then drive the stored RHFIELD_ANIMATION_ID.
                        // FX targets instead force the target sprite
                        // animation/progression immediately.
                        Command::PlayAnim
                        | Command::PlayAnimLoop
                        | Command::PlayAnimFreeze
                        | Command::PlayAnimFrozen => {
                            let anim = match elem.get_property(crate::sequence::Field::AnimationId)
                            {
                                Some(crate::sequence::FieldValue::Animation(anim)) => Some(*anim),
                                Some(crate::sequence::FieldValue::Integer(v)) => {
                                    crate::order::OrderType::try_from(*v).ok()
                                }
                                _ => None,
                            };
                            let Some(anim) = anim else {
                                tracing::warn!(
                                    entity = ?owner,
                                    cmd = ?cmd,
                                    "PlayAnim*: missing/invalid AnimationId — terminating",
                                );
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                                continue;
                            };

                            let Some(owner_entity) = self.get_entity(owner) else {
                                self.sequence_manager.element_impossible(seq_id, elem_idx);
                                continue;
                            };
                            if owner_entity.is_human() {
                                let mut order =
                                    crate::order::Order::new(anim, 0.0, 0.0, self.alloc_order_id());
                                order.compute_direction = false;
                                self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                                self.sequence_manager.element_in_progress(seq_id, elem_idx);
                                continue;
                            }

                            let is_fx_target = owner_entity.kind().is_fx_target();
                            if !is_fx_target {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                                continue;
                            }

                            // Progression tags — stored as raw u32
                            // ordinal on `TargetData.progression`,
                            // matching the `FrameProgression` enum.
                            let progression_ordinal = match cmd {
                                Command::PlayAnim => {
                                    crate::sprite::FrameProgression::Default as u32
                                }
                                Command::PlayAnimLoop => {
                                    crate::sprite::FrameProgression::Cyclically as u32
                                }
                                Command::PlayAnimFreeze => {
                                    crate::sprite::FrameProgression::FreezeWhenTerminated as u32
                                }
                                Command::PlayAnimFrozen => {
                                    crate::sprite::FrameProgression::FrozenLastFrame as u32
                                }
                                _ => unreachable!(),
                            };
                            if let Some(entity) = self.get_entity_mut(owner) {
                                let direction = entity.element_data().direction() as u16;
                                if let crate::element::Entity::Target(t) = entity {
                                    t.target.progression = progression_ordinal;
                                }
                                let sprite = &mut entity.element_data_mut().sprite;
                                // Scripts occasionally address FX
                                // targets with actor-only animations
                                // (e.g. TG_Panel +
                                // TransitionSittingWaitingUpright);
                                // log and skip rather than panic.
                                if sprite.has_animation(anim) {
                                    sprite.force_animation(anim, direction);
                                    sprite.reset_sprite_frame(false);
                                } else {
                                    tracing::warn!(
                                        ?owner,
                                        ?anim,
                                        profile = %sprite.frame_profile_name,
                                        "PlayAnim*: animation unmapped for this sprite profile — skipping",
                                    );
                                }
                            }
                            self.sequence_manager.element_terminated(seq_id, elem_idx);
                        }

                        // PC-side target interaction commands.  Each
                        // enqueues a per-command animation order on
                        // the PC (USING_LEVER / HITTING_TARGET /
                        // HANDLING_TARGET / TAKING_TARGET /
                        // SEARCHING), and on DONE the engine launches
                        // the corresponding `Activate*` interaction
                        // element on the target antagonist.
                        //
                        // The order driver plays the PC order first;
                        // `apply_pc_target_interaction_side_effect`
                        // launches the target activation when that
                        // order reports `MotionState::Done`.
                        Command::HitTarget
                        | Command::HandleTarget
                        | Command::UseLever
                        | Command::TakeTarget
                        | Command::SearchCmd => {
                            let antagonist = match &elem.data {
                                crate::sequence::SequenceElementData::Interaction {
                                    antagonist,
                                } => *antagonist,
                                _ => None,
                            };
                            let Some(target_id) = antagonist else {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                                continue;
                            };
                            // Only FX targets route through the
                            // script dispatcher. `SearchCmd` on a
                            // corpse and `UseLever` on a mobile take
                            // different paths that aren't handled here.
                            let antag_is_fx_target = self
                                .get_entity(target_id)
                                .is_some_and(|e| e.kind().is_fx_target());
                            if !antag_is_fx_target {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                                continue;
                            }
                            let anim_type = match cmd {
                                Command::HitTarget => crate::order::OrderType::HittingTarget,
                                Command::HandleTarget => crate::order::OrderType::HandlingTarget,
                                Command::UseLever => crate::order::OrderType::UsingLever,
                                Command::TakeTarget => crate::order::OrderType::TakingTarget,
                                Command::SearchCmd => crate::order::OrderType::Searching,
                                _ => unreachable!(),
                            };
                            let order = crate::order::Order::new(
                                anim_type,
                                0.0,
                                0.0,
                                self.alloc_order_id(),
                            )
                            .with_antagonist(target_id);
                            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                            self.sequence_manager.element_in_progress(seq_id, elem_idx);
                        }

                        _ => {
                            // Dispatch for remaining owner-instructed
                            // commands will be added per-command;
                            // marking terminated here keeps the
                            // sequence ticking.  Warn so unhandled
                            // commands don't silently vanish (the
                            // Seek-vs-Move bug hid here for months
                            // because the element just terminated
                            // without any log — Seek needed dispatch
                            // through the Move path and this default
                            // arm swallowed it).
                            tracing::warn!(
                                ?cmd,
                                ?owner,
                                ?seq_id,
                                elem_idx,
                                "InstructOwner: no dispatch for command; terminating element"
                            );
                            self.sequence_manager.element_terminated(seq_id, elem_idx);
                        }
                    }
                }
                crate::sequence::SequenceAction::ExecuteImmediateOwner {
                    owner,
                    sequence_id: seq_id,
                    element_index: elem_idx,
                } => {
                    self.dispatch_execute_immediate_owner(
                        assets,
                        owner,
                        seq_id,
                        elem_idx,
                        &mut deferred_process_messages,
                    );
                }
                crate::sequence::SequenceAction::EngineCommand {
                    sequence_id: seq_id,
                    element_index: elem_idx,
                }
                | crate::sequence::SequenceAction::ExecuteImmediateEngine {
                    sequence_id: seq_id,
                    element_index: elem_idx,
                } => {
                    self.dispatch_engine_or_execute_immediate(
                        display,
                        assets,
                        seq_id,
                        elem_idx,
                        &mut deferred_engine_messages,
                    );
                }
            }

            // After-action drain: any `register_element_to_go` calls
            // triggered by the action's cascade may have queued
            // immediate-dispatch actions onto
            // `pending_immediate_actions`.  Splice them onto the
            // FRONT of the work queue so they fire before the next
            // action in the batch — registration's immediate side
            // effect must fire inline.
            let pending = self.sequence_manager.take_pending_immediate_actions();
            for action in pending.into_iter().rev() {
                actions.push_front(action);
            }
        }

        // ── Dispatch deferred ProcessMessage from sequence SendMessage ──
        if !deferred_process_messages.is_empty() || !deferred_engine_messages.is_empty() {
            self.dispatch_sequence_messages(
                assets,
                &deferred_process_messages,
                &deferred_engine_messages,
            );
        }

        // ── Dispatch deferred FX-target IElementTargetScript::ActivatedBy*
        // calls collected from Command::Activate* sequence elements.
        self.dispatch_target_activations(assets, &pending_target_activations);

        // ── Per-frame movement tick ─────────────────────────────
        // Advance all entities that have active paths.
        let (arrived_entities, galopp_entities) = self.tick_entity_movement(assets);

        // ── Quit swordfight with far opponents ──────────────────
        // `quit_swordfight_with_far_opponents` is called ONLY during
        // walking-with-sword movement, NOT for stationary entities.
        // Only check entities actively moving in sword state.
        {
            let ids_to_check: Vec<EntityId> = self
                .entities
                .iter()
                .enumerate()
                .filter_map(|(idx, slot)| {
                    let e = slot.as_ref()?;
                    let h = e.human_data()?;
                    if h.opponents.is_empty() {
                        return None;
                    }
                    let a = e.actor_data()?;
                    // Only check during active sword movement.
                    if !matches!(
                        a.action_state,
                        crate::element::ActionState::MovingSword
                            | crate::element::ActionState::MovingFastSword
                    ) {
                        return None;
                    }
                    Some(EntityId(idx as u32))
                })
                .collect();
            for eid in ids_to_check {
                self.quit_swordfight_with_far_opponents(assets, eid);
            }
        }

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
        {
            let pinch_aborts: Vec<(crate::sequence::SequenceId, usize)> = self
                .entities
                .iter()
                .enumerate()
                .filter_map(|(idx, slot)| {
                    let e = slot.as_ref()?;
                    if !e.is_pc() {
                        return None;
                    }
                    let a = e.actor_data()?;
                    if !matches!(
                        a.action_state,
                        crate::element::ActionState::MovingSword
                            | crate::element::ActionState::MovingFastSword
                    ) {
                        return None;
                    }
                    let seq_id = a.active_movement.sequence_id?;
                    let elem_idx = a.active_movement.element_index;
                    if !e.position_iface().is_moving_map() {
                        return None;
                    }
                    let eid = EntityId(idx as u32);
                    if !crate::engine::melee::enemies_are_blocking_my_movement(&self.entities, eid)
                    {
                        return None;
                    }
                    Some((seq_id, elem_idx))
                })
                .collect();
            for (seq_id, elem_idx) in pinch_aborts {
                self.sequence_manager.element_impossible(seq_id, elem_idx);
            }
        }

        // ── Dispatch EventReachPoint to NPCs that just finished walking ──
        // Fires `Think(EVENT_REACHPOINT)` when a MOVE sequence
        // element terminates.
        if !arrived_entities.is_empty() {
            self.dispatch_reach_point_events(assets, &arrived_entities);
        }

        // ── Dispatch EventGaloppLoopEnd to riders with RIDER_CHARGE flag ──
        // When a rider's running animation reaches half/end frame
        // with RIDER_CHARGE, fire `Think(EVENT_GALOPP_LOOP_END)` so
        // the AI can check whether to begin the actual charge pass.
        if !galopp_entities.is_empty() {
            self.dispatch_galopp_loop_events(assets, &galopp_entities);
        }

        // ── Per-frame zone occupant update ─────────────────────
        // After movement, check actors against script zone polygons.
        // Fires EnterZone/ExitZone on zone scripts when occupancy changes.
        self.tick_zone_occupants(assets);

        // ── Per-frame animation tick ────────────────────────────
        // Advance sprite animations for idle actors, FX, and other entities.
        // Moving actors are animated inside tick_entity_movement().
        // Advance line-jump sequences: interpolate 3D position for
        // actors currently mid-jump.  Runs before the animation tick
        // so the sprite drawn this frame reflects the new position.
        self.tick_active_jumps(assets);

        // Lazily reassert the "actor with no current order has a
        // pending Wait" invariant before the idle animation driver
        // reads `current_order_for_actor`, otherwise an actor that
        // just lost its final element can keep displaying the
        // previous movement/transition sprite row.
        self.ensure_wait_elements_for_idle_actors();

        // ── PC `Execute` per-arm validity pre-tick gate ─────────
        // Run the init-phase validity guards for TAKING / EATING /
        // SEARCHING / HEALING / HELPING-CLIMB transitions /
        // corpse-carry transitions / jump-init arms before the
        // animation driver so failing init-phase arms are aborted /
        // terminated synchronously instead of running their first
        // frame and then being marked Impossible from inside the
        // entity-iter borrow.
        self.pre_tick_pc_execute_validity(assets);

        let (_ai_anim_done, combat_injury_terminated, anim_outcomes) =
            self.tick_entity_animations(assets);
        // Process sequence-element / door-pass animation completions
        // collected this tick (Turn, UnlockDoor, door-pass Transition).
        self.process_anim_completion_outcomes(anim_outcomes, assets);
        // Dispatch EventAfterCombatInjury when a combat-hit /
        // stunned / weak animation terminates on a soldier.
        for entity_id in combat_injury_terminated {
            self.dispatch_ai_stimulus(
                entity_id,
                crate::ai::Stimulus::new(crate::ai::StimulusType::EventAfterCombatInjury),
            );
        }

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

        // ── Per-actor script ActionChange dispatch ─────────────
        // After all animations have been updated, check for changes
        // and dispatch ActionChange(newAction, oldAction) to per-actor
        // scripts via the `set_animation` callback.
        self.dispatch_actor_action_changes(assets);

        // ── Per-scroll script Hourglass dispatch ────────────────
        // Every active scroll with a bound script bumps a per-scroll
        // `script_hourglass_timeout` counter; on every 25th active
        // frame the scroll's `IScrollScript::Hourglass(0)` fires
        // (bracketed by `SetScrollExecutingScript` / reset).
        self.dispatch_scroll_hourglasses(assets);

        // ── Per-frame NPC view refresh ─────────────────────────
        // Update each NPC's vision cone (direction, aperture,
        // radius) from head turning, lean-out, stare, drunk wobble,
        // death fade.  Must run before `tick_enemy_ai` so the
        // detection pass sees the current cone parameters.
        // ── Deferred body-broadcast from downed NPCs ────────────
        // NPCs whose `inform_my_friends` flag was set by
        // `set_concussion_of_the_brain` broadcast DETECTABLE_BODY to
        // every ally during Hourglass.
        self.tick_inform_my_friends();

        // ── Deferred resurrection-broadcast + eye-status apply ──
        // Mirror of the fan-out above, but for NPCs that just came
        // back up (civilian EVENT_FITAGAIN).  Remove the risen NPC
        // from every friend's DETECTABLE_BODY list and flip their
        // own `eye_status` back to `LookForward`.
        self.tick_ai_pending_resurrection_and_eyes();

        self.refresh_npc_views();

        // ── Per-tick AILOCK_BUSY edge detector ─────────────────
        // Lock or unlock AILOCK_BUSY based on the live
        // `is_very_very_busy` predicate (posture or active PassDoor /
        // Fall element).  Runs after the view refresh.
        self.tick_npc_busy_edge_detect();

        // ── Stuck-on-ladder emergency counter ──────────────────
        // Bump per frame for non-script-locked NPCs on outdoor
        // ladders idling in CMD_WAIT/CMD_MOVE_WAITING; after 25
        // frames force a ReturnToDuty so the actor can self-recover.
        // Runs after the BUSY edge detector.
        self.tick_npc_stuck_on_ladder(assets);

        // ── Locked-frame timer bumps ───────────────────────────
        // When any lock is held the entire Hourglass tail
        // short-circuits while the three timer ring-frames
        // (`when_does_timer_ring`, `when_does_macro_timer_ring`,
        // `emoticon_expiration_date`) tick forward by +1.  This both
        // keeps the relative timer offset stable across the lock
        // window and acts as the "skip the fire" gate for the
        // downstream macro-timer / EVENT_TIMER fire checks (which
        // compare against the live `frame_counter`).
        self.tick_npc_locked_frame_timer_bumps();

        // ── Macro-timer hourglass ──────────────────────────────
        // Poll the macro-specific timer each frame and, when it
        // rings, call `execute_next_macro_command` directly —
        // bypassing the stimulus queue so CMD_WAIT / CMD_BEND
        // resume cleanly.  Runs before `tick_enemy_ai` so any
        // resulting movement-order / substate change is visible
        // to the detection pass.
        self.tick_ai_macro_timers(assets);

        // ── Per-frame enemy AI tick ─────────────────────────────
        // Vision → alert → pursue.  Stand-in for the full
        // detection / `Think(stimulus)` pipeline until the state
        // machine is ported.  Without this, enemies stand around
        // doing nothing.
        self.tick_enemy_ai(assets);

        // ── Per-frame ambush-point peek scan ────────────────────
        // Drive the Far/Near/Checked ambush-point transitions and
        // dispatch the CheckAmbushPoint left/right substate change
        // when the NPC enters LOS for the first time.
        self.tick_refresh_ambush_points(assets);

        // ── Post-AI script state-change notifications ───────────
        // Notify per-actor scripts of AI state transitions via
        // FilterAIEvent(source, AI_STATE_CHANGE_TO_*).  Return value
        // ignored — informational only.
        self.dispatch_ai_state_change_notifications(assets);

        // ── NPC speech ──────────────────────────────────────────
        // Drain pending AI remarks (set by `say` during AI ticks)
        // and dispatch to the sound manager as exclamation playback.
        self.process_npc_speech(assets);

        // ── HUD speech-log decay ────────────────────────────────
        // Decrement the per-remark display timer and evict expired
        // entries every frame regardless of `speech_display` so the
        // Vec does not grow unbounded when the overlay is off.
        self.tick_screen_remarks();

        // ── Periodic AI tasks (every 16 frames, staggered) ────
        // Stuck recovery, stalled timer restart, etc.
        self.tick_periodic_ai(assets);

        // ── Per-frame patrol coordination ──────────────────────
        // Chiefs record position history, compute formation positions
        // for their minions, and dispatch CALL_PATROL_COORDINATE.
        self.tick_patrol_coordination(assets);

        // ── Per-frame bow-shot tick ─────────────────────────────
        // Drive the `SHOOTING_WITH_BOW` animation for every actor
        // with an active bow shot; when the animation reports
        // `Done`, spawn an arrow projectile and notify the sequence
        // manager.
        self.tick_bow_shots(assets);

        // ── Per-frame arrow tick ────────────────────────────────
        // C++ `ShootWithBowAt` calls `pArrow->Hourglass()` before
        // `AddElement`, then appends the arrow immediately to
        // `marrayElements`. `PerformHourglass` checks the dynamic array
        // size each iteration, so a freshly released arrow can still be
        // visited by the normal element hourglass pass later in that
        // same frame. Keep this after bow-shot release to preserve that
        // spawn-frame double advance.
        self.tick_arrows(assets);

        // ── Per-frame purse / coin tick ─────────────────────────
        // Drive purse trajectories until impact (then burst into
        // child coins), coin trajectories until landing (then
        // broadcast DETECTABLE_OBJECT for the AI distraction hook),
        // and the purse Hourglass that despawns a purse once all
        // its child coins are taken.
        self.tick_purses_and_coins(assets);
        // NOTE: tick_purses_and_coins takes &LevelAssets (not &mut) because
        // accessory sprite attach now clones from preloaded prototypes.

        // ── Beggar-solicitation tick ────────────────────────────
        // For each PC currently in `SimulatingBeggar` posture,
        // iterate civilians and toss a coin to the beggar if a
        // donor passes the full predicate chain.
        self.tick_beggar_bids(assets);

        // ── Per-frame wasp-nest tick ────────────────────────────
        // Advance wasp-nest trajectories, burst them on impact
        // (spawning inert wasps + seeding `flying_wasp_count`),
        // emit the buzz FX each tick while wasps fly, and expire
        // each wasp after its lifetime elapses.
        self.tick_wasp_nests(assets);

        // ── Per-frame net tick ──────────────────────────────────
        // Drive falling nets along their trajectory, fire the
        // capture sweep on landing, and release victims when the
        // net expires.  See `engine/nets.rs` for what's
        // intentionally out of scope.
        self.tick_nets(assets);

        // ── Per-frame melee combat tick ─────────────────────────
        // Process sword strikes (sequence-driven and AI-driven),
        // apply damage, handle death/KO/wakeup transitions, and
        // tick concussion healing.
        self.tick_melee_combat(assets);

        // Per-frame soldier counter decrements (apple-smell).
        self.tick_apple_smell();

        // Per-frame body-direction re-snap to `primary_target` for
        // soldiers in reactiontime / bow substates.
        self.tick_soldier_track_primary_target();

        // Per-frame PC life-point auto-heal (immortal bump +
        // Easy-mode slow regen).  Runs after the
        // melee/concussion/tiredness pass.
        self.tick_pc_auto_heal();

        // ── Per-frame ability tick ─────────────────────────────
        // Drive hero ability animations (carry, tie, heal, whistle,
        // traps) and apply cross-entity effects on completion.
        self.tick_abilities(display, assets);

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

        // ── Shouldered-carry ceiling check ─────────────────────
        // If a PC carrying another PC on their shoulders walks
        // under a low ceiling, force the shouldered PC off.
        self.tick_shouldered_carry_ceiling(assets);

        // ── Carried entity position sync ───────────────────────
        // Keep bodies carried by Little John positioned on the carrier
        // and drive their sprite animation (BeingLifted/BeingCarried/
        // BeingDropped) synchronized with the carrier.  Needs the
        // campaign profile manager to look up LittleJohnCarry contextual
        // actions on the carrier.
        if self.campaign.is_some() {
            abilities::sync_carried_positions(&mut self.entities, &assets.profile_manager);
        }

        // ── Swordfight-drag IgnoreMouseEvent bracket ────────────
        // If the selected PC was swordfighting at entry to
        // `perform_hourglass` but is no longer swordfighting after
        // the per-element / sequence-manager hourglass, raise the
        // ignore-mouse-event bracket so a drag in flight when the
        // swordfight ended this tick is suppressed.  We push the
        // request as a side effect; the host gates it on
        // `InputState::is_dragging` in `apply_side_effects`.
        if was_swordfighting && !self.is_selected_pc_swordfighting() {
            self.pending_side_effects.pending_swordfight_drag_ignore = true;
        }

        // ── Titbit sync + per-frame update ──────────────────────
        // First, sync persistent titbits (emoticons, unconscious
        // stars, alert indicators) with current entity state.
        self.sync_titbits(assets);

        // Then run the titbit update to advance animations and
        // expire finished titbits.
        {
            let entities_ref = &self.entities;
            let query = EntityTitbitQuery {
                entities: entities_ref,
                sequence_manager: &self.sequence_manager,
                follow_element: self.seats[0].follow_element,
            };
            self.titbit_manager.update(&query);
            // PrepareRefresh: advance blink counter, sort by
            // display order using each supplier entity's Y position
            // as a stand-in (we don't compute display order yet).
            self.titbit_manager.prepare_refresh(|handle| {
                let idx = handle.0 as usize;
                entities_ref
                    .get(idx)
                    .and_then(|opt| opt.as_ref())
                    .map(|e| e.element_data().position_map().y)
            });
        }

        // ── Ground mark animation ────────────────────────────────
        // Deliberately NOT advanced here: ground marks only
        // increment their current sprite frame inside the per-mark
        // on-screen guard, so off-screen marks freeze and never
        // retire.  Matching that means the advancement + retirement
        // belongs in the renderer (`render_ground_marks`), which
        // knows the current view box.

        // Selection ring animation lives host-side now —
        // `Game::run_engine_tick` advances `host.selection_mark`
        // once per frame, gated on the same `should_run_hourglass`
        // check as this function, so pause / console still freeze
        // the ring.

        // ── Check selected PCs are still alive ───────────────────
        {
            let mut deselect = Vec::new();
            for &pc_id in &self.seats[0].selection {
                if let Some(Some(entity)) = self.entities.get(pc_id.0 as usize) {
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
                self.messenger.send(Message::pc(
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
        self.timer_elements.retain_mut(|timer| {
            if timer.remaining <= 1 {
                expired.push(timer.element_ref);
                false
            } else {
                timer.remaining -= 1;
                true
            }
        });
        for r in expired {
            self.sequence_manager
                .element_terminated(r.sequence_id, r.element_index);
        }

        // ── SendCondolationCard dispatch ─────────────────────────
        // Drain the per-tick queue of sequence-element-terminated
        // notifications and fire per-entity cleanup.  Runs last so
        // every sequence state change from this tick's dispatching
        // is captured.
        self.dispatch_condolations(assets);

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
        self.drain_pending_self_stimuli(assets);

        // ── One-shot mission-script `PostInitialize` ──────────────
        // Fires once on the first tick after level load.  Lives
        // sim-side, after the rest of the tick's logic, so rollback
        // replay runs it deterministically and any side effects
        // PostInit pushes land in this frame's `SideEffects`
        // bundle rather than leaking a frame late.
        self.run_post_initialize_if_needed(assets);

        // ── End-of-tick immediate-action drain ──────────────────────
        // Catch any `register_element_to_go` calls that happened
        // in post-action passes (condolation fan-out, self-stimulus
        // drains, PostInitialize, etc.) without piggybacking on the
        // hourglass action-loop drain.  Close the immediate-side-
        // effect window before returning control to the host
        // renderer so post-tick state reads see the immediate side
        // effects.
        self.drain_pending_immediate_actions_sync(display, assets);

        GameCode::LevelInProgress
    }

    // ─── Stealth command dispatch ───────────────────────────────

    /// Execute a stealth posture command (CrouchDown, CrouchUp,
    /// EnterBeggar, LeaveBeggar, LeaveSpy, LeaveTree).
    ///
    /// Validates the transition, changes posture + action state,
    /// and marks the sequence element terminated.
    fn dispatch_stealth_command(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        use crate::element::ActionState;
        use crate::stealth;

        let entity = match self.entities.get(owner.0 as usize).and_then(|s| s.as_ref()) {
            Some(e) => e,
            None => {
                self.sequence_manager.element_terminated(seq_id, elem_idx);
                return;
            }
        };

        let posture = entity.element_data().posture;
        let action_state = entity
            .actor_data()
            .map(|a| a.action_state)
            .unwrap_or(ActionState::Waiting);
        let is_swordfighting = entity
            .actor_data()
            .map(|a| a.action_state.is_sword())
            .unwrap_or(false);

        if !stealth::can_execute_stealth_command(command, posture, action_state, is_swordfighting) {
            tracing::debug!(
                ?owner,
                ?command,
                ?posture,
                ?action_state,
                "stealth command rejected: preconditions not met"
            );
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return;
        }

        let transition = match stealth::stealth_transition(command) {
            Some(t) => t,
            None => {
                self.sequence_manager.element_terminated(seq_id, elem_idx);
                return;
            }
        };

        // Resolve the HIDDEN-titbit phase from the PC's identity
        // before we take a mutable borrow on `self.entities`.
        let hidden_phase = if transition.result_posture.is_hidden() {
            let Some(Some(crate::element::Entity::Pc(pc))) = self.entities.get(owner.0 as usize)
            else {
                self.sequence_manager.element_terminated(seq_id, elem_idx);
                return;
            };
            self.campaign.as_ref().unwrap_or_else(|| {
                panic!("dispatch_stealth_command: campaign missing for entity {owner:?}")
            });
            let profile = assets
                .profile_manager
                .get_character(pc.pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "dispatch_stealth_command: PC entity {} has unknown profile_index {}",
                        owner.0, pc.pc.profile_index
                    )
                });
            Some(crate::titbit::HiddenCharacter::for_pc(pc.pc.robin, &profile.filename).to_phase())
        } else {
            None
        };

        // Apply posture + action state change, queue the transition
        // animation: the dispatch registers a transition sequence
        // element whose `animation` maps to an order, and the order
        // drives the sprite animation.
        if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize) {
            let old_posture = entity.element_data().posture;
            entity.set_posture(transition.result_posture);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = transition.result_action_state;
            }
            self.push_new_order(seq_id, elem_idx, transition.animation, 0.0, 0.0);
            tracing::debug!(
                ?owner,
                ?command,
                posture = ?transition.result_posture,
                animation = ?transition.animation,
                "stealth transition applied"
            );

            // HIDDEN titbit lifecycle: add on entering Spy/Tree,
            // remove on leaving.
            use crate::position_interface::Point3D;
            use crate::titbit::{ElementHandle, TitbitKind};
            let handle = ElementHandle(owner.0);
            if transition.result_posture.is_hidden() && !old_posture.is_hidden() {
                self.titbit_manager.add_titbit(
                    Point3D::default(),
                    0,
                    TitbitKind::Hidden,
                    handle,
                    hidden_phase.expect("hidden_phase resolved above when entering hidden posture"),
                    handle, // element_manager
                    false,  // run
                    0,      // forced_id (auto)
                    true,   // display_titbits_enabled
                    None,   // supplier_display_order
                    None,   // supplier_layer
                );
            } else if !transition.result_posture.is_hidden() && old_posture.is_hidden() {
                self.titbit_manager
                    .remove_titbit(TitbitKind::Hidden, handle);
            }

            // Beggar-disguise near-coin flag toggle.  The original
            // toggles the flag on the
            // TRANSITION_WAITING_UPRIGHT_SIMULATING_BEGGAR animation
            // DONE (and `false` on the reverse transition).  We snap
            // the posture at command-dispatch time (the actual anim
            // plays out of `order_queue`), so the flag toggle moves
            // here where the posture change is authoritative.
            if transition.result_posture == crate::element::Posture::SimulatingBeggar
                && old_posture != crate::element::Posture::SimulatingBeggar
            {
                self.set_beggar_flags_of_near_coins_on_ground(owner, true);
            } else if old_posture == crate::element::Posture::SimulatingBeggar
                && transition.result_posture != crate::element::Posture::SimulatingBeggar
            {
                self.set_beggar_flags_of_near_coins_on_ground(owner, false);
            }
        }

        self.sequence_manager.element_terminated(seq_id, elem_idx);
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

        let posture = match self.entities.get(owner.0 as usize).and_then(|s| s.as_ref()) {
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

        if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize) {
            entity.set_posture(transition.result_posture);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = transition.result_action_state;
            }
        }
        if queue_anim {
            // `compute_direction = false` on the transition
            // order — direction is preserved so the soldier
            // finishes facing the same way it was leaning.
            let mut order =
                crate::order::Order::new(transition.animation, 0.0, 0.0, self.alloc_order_id());
            order.compute_direction = false;
            if let Some((seq_id, elem_idx)) = dispatching {
                self.sequence_manager.push_order_on(seq_id, elem_idx, order);
            } else {
                // No dispatching element found — spawn a single-
                // order generic sequence so the visible unstick
                // transition still plays.  Without a host element
                // we launch a tiny one just to carry this animation.
                self.launch_single_order_sequence_stamped(owner, Command::Generic, order);
            }
        }

        // Set `posture_after_transition` so downstream dispatch
        // (e.g. `dispatch_attentive_transition`) decides whether to
        // run the command's real transition or snap.
        if let Some((seq_id, elem_idx)) = dispatching
            && let Some(elem) = self.sequence_manager.get_element_mut(seq_id, elem_idx)
        {
            elem.posture_after_transition = transition.result_posture;
            elem.action_state_after_transition = transition.result_action_state;
        }

        // Remove HIDDEN titbit when leaving a hidden posture.
        if posture.is_hidden() {
            self.titbit_manager
                .remove_titbit(TitbitKind::Hidden, ElementHandle(owner.0));
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
        self.sequence_manager
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

    pub(super) fn apply_door_pass_continue_state(
        &mut self,
        entity_id: EntityId,
        action: crate::order::OrderType,
    ) {
        use crate::element::{ActionState, Posture};
        use crate::order::OrderType as OT;

        let posture = match action {
            OT::ClimbingWallUp
            | OT::ClimbingWallDown
            | OT::ClimbingWallUpFast
            | OT::ClimbingWallDownFast => Some(Posture::OnWall),
            OT::ClimbingLadderUp
            | OT::ClimbingLadderDown
            | OT::ClimbingLadderUpFast
            | OT::ClimbingLadderDownFast => Some(Posture::OnLadder),
            OT::WalkingCrouched => Some(Posture::Crouched),
            OT::WalkingUpright
            | OT::WalkingAlerted
            | OT::WalkingStairs
            | OT::RunningStairs
            | OT::RunningUpright => Some(Posture::Upright),
            _ => None,
        };
        let Some(posture) = posture else {
            return;
        };

        let lift_direction = self
            .get_entity(entity_id)
            .and_then(|entity| entity.element_data().sector())
            .and_then(|sector| {
                self.grid_sector_by_number(crate::sector::SectorNumber::new(
                    u16::from(sector) as i16
                ))
            })
            .and_then(|sector| match (posture, sector.lift_type) {
                (Posture::OnWall, Some(crate::sector::LiftType::Wall))
                | (Posture::OnLadder, Some(crate::sector::LiftType::Ladder)) => {
                    Some(sector.lift_direction)
                }
                _ => None,
            });

        let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize) else {
            return;
        };
        if entity.actor_data().is_none() {
            return;
        }

        entity.set_posture(posture);
        if let Some(dir) = lift_direction {
            entity.element_data_mut().set_direction_instantly(dir);
        }
        let action_state = match action {
            OT::RunningUpright
            | OT::RunningStairs
            | OT::ClimbingWallUpFast
            | OT::ClimbingWallDownFast
            | OT::ClimbingLadderUpFast
            | OT::ClimbingLadderDownFast => ActionState::MovingFast,
            _ => ActionState::Moving,
        };
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = action_state;
        }
    }

    pub(super) fn apply_door_pass_transition_done_side_effects(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        use crate::element::{ActionState, Point2D, Posture};
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

        let Some((layer_in, layer_out, sector_in, sector_out, point_in, point_mid, point_out)) =
            self.mission_script
                .as_ref()
                .and_then(|s| s.game_host())
                .and_then(|host| host.doors.get(usize::from(door_index)))
                .map(|door| {
                    (
                        door.layer_in,
                        door.layer_out,
                        door.sector_in,
                        door.sector_out,
                        Point2D {
                            x: door.point_in.0,
                            y: door.point_in.1,
                        },
                        Point2D {
                            x: door.point_mid.0,
                            y: door.point_mid.1,
                        },
                        Point2D {
                            x: door.point_out.0,
                            y: door.point_out.1,
                        },
                    )
                })
        else {
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

        match action {
            OT::TransitionWaitingUprightClimbingWallUp => {
                if let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize) {
                    entity.set_posture(Posture::OnWall);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                }
            }
            OT::TransitionWaitingCrouchedClimbingWallDown => {
                if let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize) {
                    entity.set_posture(Posture::OnWall);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                }
                self.set_transition_position_map_and_compute_position_all(
                    assets, entity_id, point_in,
                );
            }
            OT::TransitionWaitingCrouchedClimbingWallDownCrenel => {
                let obstacle = self.find_projection_area_at(
                    assets,
                    layer_in,
                    u16::from(sector_in),
                    crate::geo2d::pt(point_in.x, point_in.y),
                );
                if obstacle.is_none() {
                    tracing::warn!(
                        ?entity_id,
                        ?door_index,
                        layer = layer_in,
                        sector = u16::from(sector_in),
                        point_x = point_in.x,
                        point_y = point_in.y,
                        "missing door input projection area for crenel climb-down transition"
                    );
                }
                self.set_obstacle_and_material(assets, entity_id, obstacle);
                if let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize) {
                    entity.set_posture(Posture::OnWall);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                    let elem = entity.element_data_mut();
                    if let Some(dir) = lift_direction {
                        elem.set_direction_instantly(dir);
                    }
                    let pi = &mut elem.sprite.position_iface;
                    pi.set_position_map(crate::geo2d::pt(point_in.x, point_in.y));
                    pi.new_move();
                    pi.compute_increment_all(false);
                    elem.update_grid_cell();
                }
            }
            OT::TransitionClimbingWallUpWaitingCrouched => {
                if let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize) {
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
                    assets, entity_id, point_mid,
                );
            }
            OT::TransitionClimbingWallUpWaitingCrouchedCrenel => {
                let obstacle = self.find_projection_area_at(
                    assets,
                    layer_out,
                    u16::from(sector_out),
                    crate::geo2d::pt(point_out.x, point_out.y),
                );
                if obstacle.is_none() {
                    tracing::warn!(
                        ?entity_id,
                        ?door_index,
                        layer = layer_out,
                        sector = u16::from(sector_out),
                        point_x = point_out.x,
                        point_y = point_out.y,
                        "missing door output projection area for crenel climb-up transition"
                    );
                }
                self.set_obstacle_and_material(assets, entity_id, obstacle);
                if let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize) {
                    entity.set_posture(Posture::Flying);
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.action_state = ActionState::Moving;
                    }
                    {
                        let pi = &mut entity.element_data_mut().sprite.position_iface;
                        pi.set_position_map(crate::geo2d::pt(point_mid.x, point_mid.y));
                        pi.set_old_position_map(crate::geo2d::pt(point_out.x, point_out.y));
                        pi.set_position_goal_map(crate::geo2d::pt(point_out.x, point_out.y));
                        pi.compute_increment_all(true);
                    }
                    entity.element_data_mut().update_grid_cell();
                }
            }
            OT::TransitionClimbingWallDownWaitingUpright => {
                if let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize) {
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
        point: crate::element::Point2D,
    ) {
        if let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize) {
            let elem = entity.element_data_mut();
            elem.set_position_map(point);
            elem.update_grid_cell();
        }
        let layer = self
            .get_entity(entity_id)
            .map(|entity| entity.element_data().layer())
            .unwrap_or(0);
        let obstacle =
            self.find_plane_obstacle_at(assets, layer, crate::geo2d::pt(point.x, point.y));
        if obstacle.is_none() {
            tracing::warn!(
                entity = ?entity_id,
                x = point.x,
                y = point.y,
                layer,
                "wall transition ComputePositionAll found no projection-area obstacle"
            );
        }
        self.set_obstacle_and_material(assets, entity_id, obstacle);
    }

    fn apply_door_pass_transition_completion_side_effects(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        use crate::element::{ActionState, Point2D, Posture};
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
            let game_host = self
                .mission_script
                .as_mut()
                .and_then(|s| s.game_host_mut())?;
            let door = game_host.doors.get(usize::from(door_index))?;
            let snap = match action {
                OT::TransitionWaitingUprightClimbingWallUp => Some(Point2D {
                    x: door.point_mid.0,
                    y: door.point_mid.1,
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
                assets, entity_id, snap_point,
            );
        }

        let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize) else {
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
    /// [`EngineInner::tick_entity_animations`] for non-`EventDone`
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
        outcomes: super::animation::AnimCompletionOutcomes,
        assets: &LevelAssets,
    ) {
        use super::movement::DoorPassAdvance;

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
                self.alloc_order_id(),
            )
            .with_completion(crate::order::OrderCompletion::WaspStruggleCycle { cycles_remaining });
            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
            self.do_next_order(seq_id, elem_idx);
        }

        for (seq_id, elem_idx) in outcomes.seq_terminate {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
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
            self.sequence_manager.launch_element(elem);
        }

        // ABORTED motion result: set the sequence element to
        // IMPOSSIBLE.
        for (seq_id, elem_idx) in outcomes.seq_impossible {
            self.sequence_manager.element_impossible(seq_id, elem_idx);
        }

        for (door_id, seq_id, elem_idx) in outcomes.unlock_door {
            if let Some(game_host) = self.mission_script.as_mut().and_then(|s| s.game_host_mut())
                && let Some(door) = game_host.doors.get_mut(usize::from(door_id))
            {
                door.locked_pc = false;
                tracing::debug!(
                    door_id = %door_id,
                    "UnlockDoor: lockpick animation complete, door unlocked"
                );
            }
            self.sequence_manager.element_terminated(seq_id, elem_idx);
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
                let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize) else {
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
                    &mut self.next_order_id,
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
                self.execute_pass_door(assets, eid, door_index, direct, trigger_num);
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
            if let Some((seq_id, elem_idx)) =
                self.sequence_manager.current_element_for_actor(entity_id)
            {
                match advance.clone() {
                    DoorPassAdvance::Continue {
                        destination,
                        action,
                        reverse,
                        compute_direction,
                        tolerance,
                    } => {
                        let order_id = self.alloc_order_id();
                        let mut order = crate::order::Order::new(
                            action,
                            destination.x,
                            destination.y,
                            order_id,
                        );
                        order.reverse = reverse;
                        order.compute_direction = compute_direction;
                        order.tolerance = tolerance;
                        tracing::debug!(
                            entity = ?entity_id,
                            ?action,
                            target_x = destination.x,
                            target_y = destination.y,
                            "DoorPass: resumed with movement order after transition"
                        );
                        self.apply_door_pass_continue_state(entity_id, action);
                        self.sequence_manager.push_order_on(seq_id, elem_idx, order);
                        self.do_next_order(seq_id, elem_idx);
                    }
                    DoorPassAdvance::Paused { transition_order } => {
                        self.sequence_manager
                            .push_order_on(seq_id, elem_idx, transition_order);
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
                    self.sequence_manager
                        .element_terminated(seq_id, am.element_index);
                }
                self.dispatch_reach_point_events(assets, &[entity_id]);
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
            self.titbit_manager.remove_titbit(
                crate::titbit::TitbitKind::Hidden,
                crate::titbit::ElementHandle(entity_id.0),
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
                let pos = entity.element_data().position_map().to_geo_point();
                let weapon1 =
                    super::melee::weapon_material_from_profile(entity, &assets.profile_manager);
                (target_id, pos, weapon1)
            };
            let weapon2 = self
                .get_entity(target_id)
                .map(|e| super::melee::weapon_material_from_profile(e, &assets.profile_manager))
                .unwrap_or(crate::profiles::WeaponMaterial::SteelAndWood);
            self.pending_side_effects
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
            if let Some(Some(entity)) = self.entities.get_mut(antag.0 as usize) {
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
            let Some(target_entity) = self.get_entity(target) else {
                tracing::warn!(
                    ?rescuer,
                    ?target,
                    "WakingUp DONE but antagonist target is missing"
                );
                continue;
            };
            if !target_entity.is_human() {
                tracing::warn!(
                    ?rescuer,
                    ?target,
                    "WakingUp DONE antagonist target is not human"
                );
                continue;
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
                self.apply_concussion(assets, target, 0, false);
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
        //   default ammo-bonus fall-through.  The same helper drives
        //   the per-tick proximity auto-pickup.
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
                self.entities
                    .get(object.0 as usize)
                    .and_then(|s| s.as_ref()),
                Some(crate::element::Entity::Scroll(_))
            );
            if is_scroll {
                self.scroll_is_taken(assets, object, taker);
                continue;
            }

            let object_type = self
                .entities
                .get(object.0 as usize)
                .and_then(|s| s.as_ref())
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
                    self.take_scroll(taker, object);
                }
                Some(obj_type) if taker_is_pc => {
                    // Snapshot the object's position/layer/quantity/
                    // associated-action so `apply_pc_take_object` gets
                    // the same inputs the auto-pickup path passes.
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
                        if let Some(Some(entity)) = self.entities.get_mut(taker.0 as usize)
                            && let Some(npc) = entity.npc_data_mut()
                        {
                            npc.money = npc.money.saturating_add(value);
                        }
                        // Deactivate the object (clearing `active`
                        // is our equivalent of unlinking from the
                        // engine's active-element list).
                        if let Some(Some(entity)) = self.entities.get_mut(object.0 as usize) {
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
                .entities
                .get(soldier.0 as usize)
                .and_then(|s| s.as_ref())
                .and_then(|e| e.soldier_data())
                .map(|sd| sd.soldier_profile_index);
            let beer = profile_idx
                .and_then(|idx| assets.profile_manager.get_soldier(idx))
                .map(|prof| prof.beer)
                .unwrap_or(0);
            if beer == 0 {
                continue;
            }
            if let Some(Some(entity)) = self.entities.get_mut(soldier.0 as usize)
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
                .entities
                .get(victim.0 as usize)
                .and_then(|s| s.as_ref())
                .and_then(|e| e.npc_data())
                .map(|n| n.money)
                .unwrap_or(0);
            if stolen == 0 {
                continue;
            }
            if let Some(Some(entity)) = self.entities.get_mut(victim.0 as usize)
                && let Some(npc) = entity.npc_data_mut()
            {
                npc.money = 0;
            }
            if let Some(Some(entity)) = self.entities.get_mut(thief.0 as usize)
                && let Some(npc) = entity.npc_data_mut()
            {
                npc.money = npc.money.saturating_add(stolen);
            }
        }

        // GETTING_FREE_FROM_WASP START — `Say(REMARK_WASP_STING)`.
        // Plain `say` on the AI base.
        for speaker in sides.wasp_sting_remark {
            if let Some(Some(entity)) = self.entities.get_mut(speaker.0 as usize)
                && let Some(npc) = entity.npc_data_mut()
                && let Some(base) = npc.ai_brain.base_mut()
            {
                base.say(crate::ai::Remark::WaspSting);
            }
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
            // an immutable borrow on `self.entities` across the
            // mutable `npc.ai_brain.enemy_mut()` call.
            let is_shield_bearer = self
                .entities
                .get(speaker.0 as usize)
                .and_then(|s| s.as_ref())
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
            if let Some(Some(entity)) = self.entities.get_mut(speaker.0 as usize)
                && let Some(npc) = entity.npc_data_mut()
                && let Some(enemy) = npc.ai_brain.enemy_mut()
            {
                enemy.make_special_action_remark(is_shield_bearer);
            }
        }

        // LYING_STUCK_UNDER_NET 1/31 cycle — NPCs say
        // `UnderNet` (soldier) or `CivUnderNet` (civilian) plus a
        // HEEELP noise at the entity's 2D position (volume
        // `NOISE_VOLUME_HEEELP`, = 200).
        for speaker in sides.cry_for_help_under_net {
            let (remark, origin, layer, elevation) = {
                let Some(Some(entity)) = self.entities.get(speaker.0 as usize) else {
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
                    elem.position_map().to_geo_point(),
                    elem.layer(),
                    pos3d.z.max(0.0) as u16,
                )
            };
            if let Some(Some(entity)) = self.entities.get_mut(speaker.0 as usize)
                && let Some(npc) = entity.npc_data_mut()
                && let Some(base) = npc.ai_brain.base_mut()
            {
                base.say(remark);
            }
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
    /// Wrapper around the immediate-action helpers.
    ///
    /// Dispatches the immediate side effect synchronously rather
    /// than queuing it.  Used both by `perform_hourglass_inner`'s
    /// action loop and by
    /// [`Self::drain_pending_immediate_actions_sync`] to fire
    /// `pending_immediate_actions` queued by
    /// `register_element_to_go` outside the hourglass dispatch
    /// loop.
    fn dispatch_immediate_action(
        &mut self,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
        action: crate::sequence::SequenceAction,
        deferred_process_messages: &mut Vec<(i32, i32, i32, i32)>,
        deferred_engine_messages: &mut Vec<(i32, i32, i32)>,
    ) {
        match action {
            crate::sequence::SequenceAction::ExecuteImmediateOwner {
                owner,
                sequence_id,
                element_index,
            } => self.dispatch_execute_immediate_owner(
                assets,
                owner,
                sequence_id,
                element_index,
                deferred_process_messages,
            ),
            crate::sequence::SequenceAction::ExecuteImmediateEngine {
                sequence_id,
                element_index,
            } => self.dispatch_engine_or_execute_immediate(
                display,
                assets,
                sequence_id,
                element_index,
                deferred_engine_messages,
            ),
            other => panic!(
                "dispatch_immediate_action called with non-immediate variant: {:?}",
                other
            ),
        }
    }

    /// Synchronous drain of `SequenceManager::pending_immediate_actions`.
    ///
    /// External entry points around the manager
    /// (`launch_sequence`, `launch_element`, `element_terminated`,
    /// `element_impossible`, `element_in_progress`,
    /// `element_interrupted`, `terminate_sequence`, `stop_owner`,
    /// `stop_pending_elements*`, `cancel_pending_move_commands`)
    /// can register elements via `register_element_to_go`, which in
    /// turn queues immediate `SequenceAction`s for the
    /// `ExecutedImmediately()` command groups.  Engine-side wrappers
    /// that have access to `&LevelAssets` call this helper after
    /// invoking such an entry point so the synchronous dispatch
    /// fires the same frame as the registration.
    ///
    /// `SendMessage` immediates produce `ProcessMessage` script calls
    /// that need to run after the sequence-manager state settles; we
    /// buffer them in a local `(handle, msg, arg1, arg2)` queue and
    /// flush via `dispatch_sequence_messages` once the action loop
    /// drains, mirroring the in-hourglass deferral.
    pub(crate) fn drain_pending_immediate_actions_sync(
        &mut self,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
    ) {
        if !self.sequence_manager.has_pending_immediate_actions() {
            return;
        }
        let mut deferred_process_messages: Vec<(i32, i32, i32, i32)> = Vec::new();
        let mut deferred_engine_messages: Vec<(i32, i32, i32)> = Vec::new();
        loop {
            let actions = self.sequence_manager.take_pending_immediate_actions();
            if actions.is_empty() {
                break;
            }
            for action in actions {
                self.dispatch_immediate_action(
                    display,
                    assets,
                    action,
                    &mut deferred_process_messages,
                    &mut deferred_engine_messages,
                );
            }
        }
        if !deferred_process_messages.is_empty() || !deferred_engine_messages.is_empty() {
            self.dispatch_sequence_messages(
                assets,
                &deferred_process_messages,
                &deferred_engine_messages,
            );
        }
    }

    /// Extracted from the `ExecuteImmediateOwner` match arm in
    /// `perform_hourglass_inner`.  Dispatches the owner-immediate
    /// command group (Teleport, LockAi, UnlockAi, ReplaceAnim,
    /// RestoreAnim, Speak, StartMobile, StopMobile, ActivateMobile,
    /// DeactivateMobile, Unblip, owner-bound SendMessage).
    fn dispatch_execute_immediate_owner(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        deferred_process_messages: &mut Vec<(i32, i32, i32, i32)>,
    ) {
        let cmd = match self.sequence_manager.get_element(seq_id, elem_idx) {
            Some(e) => e.command,
            None => return,
        };
        match cmd {
            Command::Unblip => {
                if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                    && entity.element_data().blipped
                {
                    entity.reveal_blip();
                }
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Command::SendMessage => {
                // Dispatch ProcessMessage to the owner's per-actor
                // script.
                let (msg, arg1, arg2) = self.extract_message_properties(seq_id, elem_idx);
                let handle = crate::natives::GameHost::actor_handle(owner);
                deferred_process_messages.push((handle, msg, arg1, arg2));
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Command::ReplaceAnim => {
                // Scripts use this to register per-sprite animation
                // fallbacks (e.g. Robin has no RunningWithSword,
                // so it's remapped to WalkingWithSword).
                let (old_anim, new_anim) = {
                    let elem = self.sequence_manager.get_element(seq_id, elem_idx);
                    let old = elem.and_then(|e| {
                        match e.get_property(crate::sequence::Field::OldAnimation) {
                            Some(crate::sequence::FieldValue::Integer(v)) => {
                                crate::order::OrderType::try_from(*v).ok()
                            }
                            _ => None,
                        }
                    });
                    let new = elem.and_then(|e| {
                        match e.get_property(crate::sequence::Field::NewAnimation) {
                            Some(crate::sequence::FieldValue::Integer(v)) => {
                                crate::order::OrderType::try_from(*v).ok()
                            }
                            _ => None,
                        }
                    });
                    (old, new)
                };
                if let (Some(old), Some(new)) = (old_anim, new_anim)
                    && let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                {
                    entity.element_data_mut().sprite.replace_anim(old, new);
                }
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Command::RestoreAnim => {
                let old_anim = {
                    let elem = self.sequence_manager.get_element(seq_id, elem_idx);
                    elem.and_then(
                        |e| match e.get_property(crate::sequence::Field::OldAnimation) {
                            Some(crate::sequence::FieldValue::Integer(v)) => {
                                crate::order::OrderType::try_from(*v).ok()
                            }
                            _ => None,
                        },
                    )
                };
                if let Some(old) = old_anim
                    && let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                {
                    entity.element_data_mut().sprite.restore_anim(old);
                }
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Command::Speak => {
                // NPC: `say_remark(speak_id, speak_flags)`.
                // PC:  `hero_speaking(speak_id, SPEECH_SCRIPT,
                //                     speak_variant)`.
                let (speak_id, speak_flags, speak_variant) = {
                    let elem = self.sequence_manager.get_element(seq_id, elem_idx);
                    let id =
                        elem.and_then(|e| match e.get_property(crate::sequence::Field::SpeakId) {
                            Some(crate::sequence::FieldValue::Integer(v)) => Some(*v),
                            _ => None,
                        });
                    let flags = elem.and_then(|e| {
                        match e.get_property(crate::sequence::Field::SpeakFlags) {
                            Some(crate::sequence::FieldValue::Integer(v)) => Some(*v),
                            _ => None,
                        }
                    });
                    let variant = elem.and_then(|e| {
                        match e.get_property(crate::sequence::Field::SpeakVariant) {
                            Some(crate::sequence::FieldValue::Integer(v)) => Some(*v),
                            _ => None,
                        }
                    });
                    (id, flags, variant)
                };
                let Some(speak_id) = speak_id else {
                    tracing::warn!(?owner, "Speak: missing SpeakId property — terminating");
                    self.sequence_manager.element_terminated(seq_id, elem_idx);
                    return;
                };
                let owner_is_pc = self.get_entity(owner).is_some_and(|e| e.is_pc());
                if owner_is_pc {
                    self.hero_speaking_script(
                        assets,
                        owner,
                        speak_id as u16,
                        speak_variant.map(|v| v as i32),
                    );
                } else if let Ok(remark) = crate::ai::Remark::try_from(speak_id)
                    && let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                    && let Some(ai) = entity.npc_data_mut().and_then(|n| n.ai_brain.base_mut())
                {
                    let flags_bits = speak_flags.unwrap_or(0) as u16;
                    let flags = crate::ai::SpeechFlags::from_bits_truncate(flags_bits);
                    ai.say_with_flags(remark, flags);
                } else {
                    tracing::warn!(
                        ?owner,
                        speak_id,
                        "Speak: invalid remark id or missing AI controller"
                    );
                }
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Command::Teleport => {
                // Read destination + layer + sector off the
                // movement element, snap the actor there, and spawn
                // the two 5-star bursts (old → new) at feet-to-eyes.
                // The element's `sector` field is ignored; sector +
                // layer are re-derived from the destination via
                // `get_sector_screen_accessible`.  Only the
                // destination point is read off the element here;
                // `dest_layer` is kept as a fallback for the
                // new-side star burst when the validation step
                // gives up.
                let (dest, dest_layer) = {
                    let elem = self.sequence_manager.get_element(seq_id, elem_idx);
                    match elem.map(|e| &e.data) {
                        Some(crate::sequence::SequenceElementData::Movement {
                            destination,
                            layer,
                            ..
                        }) => (Some(*destination), Some(*layer)),
                        _ => (None, None),
                    }
                };
                if let Some(dest) = dest {
                    self.cheat_used_flags |= 0x0000_0001; // CHEAT_TELEPORT

                    // `stop_owner` cleans up any in-flight
                    // movement / active element before the teleport
                    // so the actor doesn't resume pathing toward
                    // its old destination on the next tick.
                    self.stop_owner(owner, crate::sequence::SequencePriority::Normal);

                    // Snapshot old position & whether this is a PC
                    // before any mutation; also capture eyes/feet
                    // points for the old-position star burst.
                    let (old_pos, old_feet, old_eyes, is_pc) = {
                        let entity = match self.get_entity(owner) {
                            Some(e) => e,
                            None => {
                                self.sequence_manager.element_terminated(seq_id, elem_idx);
                                return;
                            }
                        };
                        let ed = entity.element_data();
                        let feet = entity.compute_feet_point();
                        let eyes = entity.compute_eyes_point(None);
                        (
                            ed.position_map(),
                            feet,
                            eyes,
                            matches!(entity, crate::element::Entity::Pc(_)),
                        )
                    };

                    let zero_teleport = (dest.x - old_pos.x).abs() < f32::EPSILON
                        && (dest.y - old_pos.y).abs() < f32::EPSILON;

                    // Helper: emit 5 UnconsciousStar titbits from
                    // feet → eyes with the canonical phases.
                    let emit_stars = |mgr: &mut crate::titbit::TitbitManager,
                                      feet: crate::element::Point3D,
                                      eyes: crate::element::Point3D,
                                      layer: u16| {
                        let feet = crate::position_interface::Point3D {
                            x: feet.x,
                            y: feet.y,
                            z: feet.z,
                        };
                        let eyes = crate::position_interface::Point3D {
                            x: eyes.x,
                            y: eyes.y,
                            z: eyes.z,
                        };
                        let inc = crate::position_interface::Point3D {
                            x: (eyes.x - feet.x) * 0.25,
                            y: (eyes.y - feet.y) * 0.25,
                            z: (eyes.z - feet.z) * 0.25,
                        };
                        let mut p = crate::position_interface::Point3D {
                            x: feet.x - 4.0,
                            y: feet.y - 4.0,
                            z: feet.z,
                        };
                        for &phase in &[4u16, 12, 20, 12, 4] {
                            mgr.add_titbit(
                                p,
                                layer,
                                crate::titbit::TitbitKind::UnconsciousStar,
                                crate::titbit::ElementHandle::INVALID,
                                phase,
                                crate::titbit::ElementHandle::INVALID,
                                false,
                                crate::titbit::INVALID_ID,
                                false,
                                None,
                                None,
                            );
                            p.x += inc.x;
                            p.y += inc.y;
                            p.z += inc.z;
                        }
                    };

                    // The old-position star burst is gated by
                    // `bstars = !set_teleport_stuff(position_map, 20)`.
                    // `set_teleport_stuff(pt_old, 20)`:
                    //   ret = (teleport_counter > 0);
                    //   if position_before_teleport == position_map:
                    //       return ret  // already snapshot, leave counter
                    //   position_before_teleport = pt_old;
                    //   max_teleport_counter = teleport_counter = 20;
                    //   return ret;
                    // `bstars` is `true` only when no prior
                    // teleport-fade is active — a re-teleport
                    // during the 20-frame fade window suppresses
                    // the second star burst.  The render-side
                    // hulk-rebuild that consumes `teleport_counter`
                    // lives in `game_render.rs::render_entities_gpu`.
                    const TELEPORT_FADE_FRAMES: u16 = 20;
                    let mut bstars = true;
                    if is_pc
                        && let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                        && let Some(pc) = entity.pc_data_mut()
                    {
                        let breturn = pc.teleport_counter > 0;
                        if pc.position_before_teleport.x == old_pos.x
                            && pc.position_before_teleport.y == old_pos.y
                        {
                            // Already snapshot at this position — keep
                            // the existing counter, return prior state.
                        } else {
                            pc.position_before_teleport = old_pos;
                            pc.max_teleport_counter = TELEPORT_FADE_FRAMES;
                            pc.teleport_counter = TELEPORT_FADE_FRAMES;
                        }
                        bstars = !breturn;
                    }
                    if is_pc
                        && !zero_teleport
                        && bstars
                        && let (Some(f), Some(e)) = (old_feet, old_eyes)
                    {
                        emit_stars(&mut self.titbit_manager, f, e, dest_layer.unwrap_or(0));
                    }

                    // Probe the destination sector via
                    // `get_sector_screen_accessible`, then nudge
                    // the actor's move-box onto a walkable cell
                    // with `find_authorized_position_toward`.
                    // When either step fails the entire apply
                    // block is skipped — the actor stays put but
                    // the new-position star burst still fires.
                    let dest_geo = crate::geo2d::pt(dest.x, dest.y);
                    let probe = self.fast_grid.get_sector_screen_accessible(dest_geo);
                    let move_box = self
                        .get_entity(owner)
                        .map(|e| *e.position_iface().get_move_box());
                    let validated =
                        if let (Some(_sector_idx), Some(sector_number), Some(move_box)) =
                            (probe.sector_idx, probe.sector, move_box)
                        {
                            let mut box_at = move_box.translated(dest_geo);
                            if self.fast_grid.find_authorized_position_toward(
                                &mut box_at,
                                dest_geo,
                                probe.layer,
                            ) {
                                let center = box_at.center();
                                let dest_pt = crate::element::Point2D {
                                    x: center.x,
                                    y: center.y,
                                };
                                let sector_handle = crate::position_interface::SectorHandle::new(
                                    u16::from(sector_number),
                                );
                                Some((dest_pt, probe.layer, sector_handle, sector_number))
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                    let final_dest_layer = if let Some(v) = validated.as_ref() {
                        Some(v.1)
                    } else {
                        dest_layer
                    };

                    if let Some((
                        final_dest,
                        final_layer,
                        final_sector_handle,
                        final_sector_number,
                    )) = validated
                    {
                        // Apply new position + layer/sector + refresh
                        // position interface + grid cell.
                        if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize) {
                            let pi = entity.position_iface_mut();
                            pi.set_position_map(crate::geo2d::pt(final_dest.x, final_dest.y));
                            let ed = entity.element_data_mut();
                            ed.set_position_map(final_dest);
                            ed.set_layer(final_layer);
                            ed.set_sector(final_sector_handle);
                            ed.update_grid_cell();
                        }

                        // Re-resolve the projection-area obstacle
                        // for the new sector + position so
                        // subsequent elevation / material queries
                        // see the landing cell rather than stale
                        // data from the source.
                        let obstacle_idx = self.find_projection_area_at(
                            assets,
                            final_layer,
                            u16::from(final_sector_number),
                            crate::geo2d::pt(final_dest.x, final_dest.y),
                        );
                        self.set_obstacle_and_material(assets, owner, obstacle_idx);

                        // Landing in a lift sector snaps posture
                        // / action-state: LIFT_LADDER →
                        // (OnLadder, Waiting); LIFT_WALL →
                        // (OnWall, Waiting); LIFT_STAIRS leaves
                        // it alone.
                        if final_sector_handle.is_some() {
                            let lift = self.get_sector_lift_type(final_sector_number);
                            match lift {
                                Some(crate::sector::LiftType::Ladder) => {
                                    if let Some(Some(entity)) =
                                        self.entities.get_mut(owner.0 as usize)
                                    {
                                        entity.set_posture(crate::element::Posture::OnLadder);
                                        if let Some(actor) = entity.actor_data_mut() {
                                            actor.action_state =
                                                crate::element::ActionState::Waiting;
                                        }
                                    }
                                }
                                Some(crate::sector::LiftType::Wall) => {
                                    if let Some(Some(entity)) =
                                        self.entities.get_mut(owner.0 as usize)
                                    {
                                        entity.set_posture(crate::element::Posture::OnWall);
                                        if let Some(actor) = entity.actor_data_mut() {
                                            actor.action_state =
                                                crate::element::ActionState::Waiting;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }

                        // If this PC carries another PC or is
                        // being carried, copy the new position /
                        // layer / sector onto the partner so the
                        // carry link stays synced after the
                        // teleport.  Full `CopyPositionMapEtc`
                        // semantics require obstacle + material
                        // (ported in
                        // `abilities.rs::sync_carried_positions`);
                        // here we apply the position/layer/sector
                        // subset directly so subsequent ticks
                        // read consistent state.
                        if is_pc {
                            let (carried, carrier) = self
                                .get_entity(owner)
                                .map(|e| {
                                    let pc = e.pc_data();
                                    let human = e.human_data();
                                    (pc.and_then(|pc| pc.carried), human.and_then(|h| h.carrier))
                                })
                                .unwrap_or((None, None));
                            for partner in [carried, carrier].into_iter().flatten() {
                                if let Some(partner_entity) = self.get_entity_mut(partner) {
                                    let pel = partner_entity.element_data_mut();
                                    pel.set_position_map(final_dest);
                                    pel.set_layer(final_layer);
                                    pel.set_sector(final_sector_handle);
                                    pel.update_grid_cell();
                                }
                            }
                        }
                    }

                    // After a layer/sector swap, refresh
                    // `update_opponents_jump_lines` for both the
                    // teleporter and any carry partner that was
                    // synced above.
                    self.update_opponents_jump_lines(assets, owner);
                    if is_pc {
                        let (carried, carrier) = self
                            .get_entity(owner)
                            .map(|e| {
                                let pc = e.pc_data();
                                let human = e.human_data();
                                (pc.and_then(|pc| pc.carried), human.and_then(|h| h.carrier))
                            })
                            .unwrap_or((None, None));
                        for partner in [carried, carrier].into_iter().flatten() {
                            self.update_opponents_jump_lines(assets, partner);
                        }
                    }

                    // New-position star burst after the snap.
                    // Gated by `is_pc && !zero_teleport &&
                    // bstars` — the same hulk-fade suppression
                    // as the old-side burst.  Fires regardless
                    // of whether the position write happened.
                    if is_pc && !zero_teleport && bstars {
                        let (new_feet, new_eyes) = match self.get_entity(owner) {
                            Some(e) => (e.compute_feet_point(), e.compute_eyes_point(None)),
                            None => (None, None),
                        };
                        if let (Some(f), Some(e)) = (new_feet, new_eyes) {
                            emit_stars(
                                &mut self.titbit_manager,
                                f,
                                e,
                                final_dest_layer.unwrap_or(0),
                            );
                        }
                    }
                }
                self.sequence_manager.element_terminated(seq_id, elem_idx);
                // `actor_wait` parks the actor in a low-priority
                // idle element after the teleport so the AI
                // re-enters its default loop instead of resuming
                // whatever command was running before.
                self.actor_wait(owner);
            }
            Command::LockAi | Command::UnlockAi => {
                // NPC AI calls `script_lock(false, true)` /
                // `script_unlock`.  PCs cannot be locked this way.
                let lock = cmd == Command::LockAi;
                if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
                    && entity.is_npc()
                {
                    let is_unconscious =
                        entity.human_data().map(|h| h.unconscious).unwrap_or(false);
                    if let Some(ai) = entity.ai_controller_mut() {
                        if lock {
                            // `script_lock` normally calls Stop()
                            // unless the active command IS LockAi.
                            // Here it is, so skip the halt —
                            // otherwise we'd cancel the very
                            // command we're dispatching.
                            ai.script_lock(false, true);
                        } else if ai.script_locked {
                            ai.script_unlock(is_unconscious);
                        }
                    }
                }
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            _ => {
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
        }
    }

    /// Stage A — extracted from the combined
    /// `EngineCommand` / `ExecuteImmediateEngine` match arm in
    /// `perform_hourglass_inner`.  Dispatches engine-side
    /// commands — both the immediate group (LockUser, UnlockUser,
    /// CameraJumpTo, Timer, ActionAvailable, CharacterAvailable,
    /// OpenScroll, ownerless SendMessage) and the non-immediate
    /// engine commands handled by the same switch (CameraGoto,
    /// ZoomLevel, LockCameraOn/Stop, DisplayMap, PlayDialog,
    /// DisplayPopupText, Freeze[All]).
    fn dispatch_engine_or_execute_immediate(
        &mut self,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        deferred_engine_messages: &mut Vec<(i32, i32, i32)>,
    ) {
        // Check for SendMessage targeting the global script.
        let cmd = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.command);
        match cmd {
            Some(Command::SendMessage) => {
                // Ownerless SendMessage dispatches
                // `IEngineScript::ProcessMessage` (global).
                let (msg, arg1, arg2) = self.extract_message_properties(seq_id, elem_idx);
                deferred_engine_messages.push((msg, arg1, arg2));
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Some(Command::LockUser) => {
                // Set `user_locked` and start dropping mouse/key
                // events.
                self.user_locked = true;
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Some(Command::UnlockUser) => {
                self.user_locked = false;
                // Drop key/button edges queued while the lock was
                // held by raising `pending_reset_input`; the host
                // drain clears ThreadedInput's pressed-key cache
                // plus the UI latch state.
                self.pending_side_effects.pending_reset_input = true;
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Some(Command::Timer) => {
                // Park the element on the timer-element list; the
                // per-frame scan in `perform_hourglass` terminates
                // it when the Timer property reaches zero.
                let frames = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.get_property(crate::sequence::Field::Timer))
                    .and_then(|v| match v {
                        crate::sequence::FieldValue::Integer(n) => Some(*n),
                        _ => None,
                    })
                    .unwrap_or(0);
                self.add_timer(
                    frames,
                    crate::sequence::SequenceElementRef::new(seq_id, elem_idx),
                );
            }
            Some(Command::CameraJumpTo) => {
                // Terminate any pending camera sequence element,
                // snap the view to the requested point, invalidate
                // background, and terminate self.
                self.terminate_prev_camera_sequence_element();
                self.seats[0].follow_element = None;
                self.seats[0].locker_active = false;
                let point = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.get_property(crate::sequence::Field::CameraPoint))
                    .and_then(|v| match v {
                        crate::sequence::FieldValue::Point2D { x, y } => {
                            Some(crate::geo2d::pt(*x, *y))
                        }
                        _ => None,
                    });
                if let Some(pos) = point {
                    // Direct assignment via
                    // `check_location_is_valid_for_camera`, no
                    // separate clamp.
                    self.cutscene_camera.view_position =
                        self.check_location_is_valid_for_camera(pos);
                    self.pending_side_effects.invalidate_background = true;
                }
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Some(Command::CameraGoto) => {
                // Terminate any previous camera sequence element,
                // stash this one as the in-progress camera element,
                // and start a slide toward the target.
                // Fast-forward snaps instantly.
                self.terminate_prev_camera_sequence_element();
                self.seats[0].follow_element = None;
                self.seats[0].locker_active = false;
                let (point, speed) = {
                    let e = self.sequence_manager.get_element(seq_id, elem_idx);
                    let p = e
                        .and_then(|e| e.get_property(crate::sequence::Field::CameraPoint))
                        .and_then(|v| match v {
                            crate::sequence::FieldValue::Point2D { x, y } => {
                                Some(crate::geo2d::pt(*x, *y))
                            }
                            _ => None,
                        });
                    let s = e
                        .and_then(|e| e.get_property(crate::sequence::Field::CameraSpeed))
                        .and_then(|v| match v {
                            crate::sequence::FieldValue::Integer(n) => Some(*n as u16),
                            _ => None,
                        })
                        .unwrap_or(0);
                    (p, s)
                };
                if self.fast_forward {
                    if let Some(pos) = point {
                        self.cutscene_camera.view_position =
                            self.check_location_is_valid_for_camera(pos);
                    }
                    self.sequence_manager.element_terminated(seq_id, elem_idx);
                } else if let Some(pos) = point {
                    // Store the raw script point as
                    // `camera_wanted`, store the centered+clamped
                    // result as `camera_slide`.
                    self.cutscene_camera.camera_wanted = pos;
                    self.cutscene_camera.camera_slide =
                        self.check_location_is_valid_for_camera(pos);
                    self.cutscene_camera.fixed_camera_speed = speed;
                    self.speed = 2.0;
                    self.speed_int = 0;
                    self.cutscene_camera.sequence_element =
                        Some(crate::sequence::SequenceElementRef::new(seq_id, elem_idx));
                } else {
                    self.sequence_manager.element_terminated(seq_id, elem_idx);
                }
            }
            Some(Command::ZoomLevel) => {
                // Terminate any previous camera sequence element,
                // record the requested zoom factor, and latch this
                // element as the in-progress camera element until
                // the zoom transition finishes.
                self.terminate_prev_camera_sequence_element();
                let zoom = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.get_property(crate::sequence::Field::CameraZoomLevel))
                    .and_then(|v| match v {
                        crate::sequence::FieldValue::Float(f) => Some(*f),
                        _ => None,
                    });
                if let Some(z) = zoom {
                    self.cutscene_camera.desired_zoom_factor = z;
                    self.cutscene_camera.sequence_element =
                        Some(crate::sequence::SequenceElementRef::new(seq_id, elem_idx));
                } else {
                    self.sequence_manager.element_terminated(seq_id, elem_idx);
                }
            }
            Some(Command::LockCameraOn) => {
                // Terminate any previous camera sequence element,
                // start following the antagonist, drop any titbit
                // locks, and terminate self.
                self.terminate_prev_camera_sequence_element();
                let target = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| match &e.data {
                        crate::sequence::SequenceElementData::Interaction { antagonist } => {
                            *antagonist
                        }
                        _ => None,
                    });
                if let Some(t) = target {
                    self.seats[0].follow_element = Some(t);
                    self.seats[0].locker_active = true;
                } else {
                    self.seats[0].follow_element = None;
                    self.seats[0].locker_active = false;
                }
                self.titbit_manager.remove_lock();
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Some(Command::LockCameraStop) => {
                self.terminate_prev_camera_sequence_element();
                self.seats[0].follow_element = None;
                self.seats[0].locker_active = false;
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Some(Command::DisplayMap) => {
                // Forwards to `Minimap::display_map(show)`.
                let show = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.get_property(crate::sequence::Field::MapDisplay))
                    .and_then(|v| match v {
                        crate::sequence::FieldValue::Bool(b) => Some(*b),
                        _ => None,
                    })
                    .unwrap_or(false);
                display.minimap.display_map(show, false);
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Some(Command::PlayDialog) => {
                // Dialog display is skipped in fast-forward;
                // always send MSG_RESET_INPUT.
                if !self.fast_forward {
                    let dialog_id = self
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .and_then(|e| e.get_property(crate::sequence::Field::DialogId))
                        .and_then(|v| match v {
                            crate::sequence::FieldValue::Integer(n) => Some(*n as i32),
                            _ => None,
                        })
                        .unwrap_or(0);
                    self.pending_side_effects.pending_dialogues.push(dialog_id);
                }
                self.messenger
                    .send(Message::new(MessageType::Simple(SimpleMessage::ResetInput)));
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Some(Command::DisplayPopupText) => {
                // Popup-scroll display is skipped in fast-forward;
                // always send MSG_RESET_INPUT.
                if !self.fast_forward {
                    let text_id = self
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .and_then(|e| e.get_property(crate::sequence::Field::PopupTextId))
                        .and_then(|v| match v {
                            crate::sequence::FieldValue::Integer(n) => Some(*n as i32),
                            _ => None,
                        })
                        .unwrap_or(0);
                    self.pending_side_effects.pending_popup_texts.push(text_id);
                }
                self.messenger
                    .send(Message::new(MessageType::Simple(SimpleMessage::ResetInput)));
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Some(Command::Freeze | Command::FreezeAll) => {
                let freeze = self
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .and_then(|e| e.get_property(crate::sequence::Field::Freeze))
                    .and_then(|v| match v {
                        crate::sequence::FieldValue::Bool(b) => Some(*b),
                        _ => None,
                    })
                    .unwrap_or(false);
                self.freeze_all = freeze;
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Some(Command::CharacterAvailable) => {
                // `SetPlayable` writes `playable` AND fires
                // `EnableCharacter` / `DisableCharacter` so the
                // portrait / selection bookkeeping kicks in.
                // Dispatch the message here too so script-driven
                // SetPlayable goes through the same selection-drop
                // + interface-hidden path as the `Deactivate`
                // native.
                let (owner, playable) = {
                    let elem = self.sequence_manager.get_element(seq_id, elem_idx);
                    let owner = elem.and_then(|e| e.owner);
                    let playable = elem
                        .and_then(|e| e.get_property(crate::sequence::Field::CharacterAvailable))
                        .and_then(|v| match v {
                            crate::sequence::FieldValue::Bool(b) => Some(*b),
                            _ => None,
                        })
                        .unwrap_or(false);
                    (owner, playable)
                };
                if let Some(o) = owner
                    && let Some(entity) = self.get_entity_mut(o)
                    && let Some(pc) = entity.pc_data_mut()
                {
                    pc.playable = playable;
                    let msg_type = if playable {
                        crate::messenger::PcMessage::EnableCharacter
                    } else {
                        crate::messenger::PcMessage::DisableCharacter
                    };
                    self.messenger
                        .send(crate::messenger::Message::pc(msg_type, Some(o)));
                }
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Some(Command::ActionAvailable) => {
                // Owner PC receives `EnableAction` /
                // `DisableAction` with the action id depending on
                // the `ActionAvailable` flag.  The messenger
                // downstream flips the portrait widget and clears
                // `valid_trajectory`.
                let (owner, action_id, available) = {
                    let elem = self.sequence_manager.get_element(seq_id, elem_idx);
                    let owner = elem.and_then(|e| e.owner);
                    let action_id = elem
                        .and_then(|e| e.get_property(crate::sequence::Field::ActionId))
                        .and_then(|v| match v {
                            crate::sequence::FieldValue::Integer(n) => Some(*n),
                            _ => None,
                        })
                        .unwrap_or(0);
                    let available = elem
                        .and_then(|e| e.get_property(crate::sequence::Field::ActionAvailable))
                        .and_then(|v| match v {
                            crate::sequence::FieldValue::Bool(b) => Some(*b),
                            _ => None,
                        })
                        .unwrap_or(false);
                    (owner, action_id, available)
                };
                if let Some(o) = owner {
                    let sub = if available {
                        crate::messenger::PcMessage::EnableAction
                    } else {
                        crate::messenger::PcMessage::DisableAction
                    };
                    self.messenger
                        .send(Message::pc_with_value(sub, Some(o), action_id));
                }
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            Some(Command::OpenScroll) => {
                // Call `scroll_is_taken` on the scroll referenced
                // by `Scroll`, passing the PC from `ScrollReader`.
                // Opens the scroll and, if a script is bound,
                // dispatches its `IsTaken` handler.
                let (scroll_id, reader_id) = {
                    let elem = self.sequence_manager.get_element(seq_id, elem_idx);
                    let scroll = elem
                        .and_then(|e| e.get_property(crate::sequence::Field::Scroll))
                        .and_then(|v| match v {
                            crate::sequence::FieldValue::Element(id) => Some(*id),
                            _ => None,
                        });
                    let reader = elem
                        .and_then(|e| e.get_property(crate::sequence::Field::ScrollReader))
                        .and_then(|v| match v {
                            crate::sequence::FieldValue::Element(id) => Some(*id),
                            _ => None,
                        });
                    (scroll, reader)
                };
                if let (Some(scroll), Some(reader)) = (scroll_id, reader_id) {
                    self.scroll_is_taken(assets, scroll, reader);
                } else {
                    tracing::warn!(
                        ?scroll_id,
                        ?reader_id,
                        "OpenScroll sequence command missing Scroll/ScrollReader property"
                    );
                }
                self.sequence_manager.element_terminated(seq_id, elem_idx);
            }
            _ => {
                // Unknown commands fall through without being
                // terminated.
            }
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
/// The RNG is drained deterministically from the installed
/// `sim_rng`, so replays reproduce the same deviation sequence.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_drunken_path_deviation(
    mut waypoints: Vec<crate::geo2d::Point2D>,
    origin: crate::geo2d::Point2D,
    blood_alcohol: u8,
    is_running: bool,
    layer: u16,
    move_box: &crate::geo2d::BBox2D,
    half_diagonal: crate::geo2d::Vec2D,
    grid: &crate::fast_find_grid::FastFindGrid,
    rng: &mut fastrand::Rng,
) -> Vec<crate::geo2d::Point2D> {
    use crate::geo2d::{Point2D, pt};

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
        let mut new_path: Vec<Point2D> = Vec::with_capacity(waypoints.len() * 2);
        let mut prev = origin;
        for next in &waypoints {
            let straight = pt(next.x - prev.x, next.y - prev.y);
            let max_norm = straight.x.abs().max(straight.y.abs());
            // Midpoint of the current segment.
            let midpoint = pt(prev.x + 0.5 * straight.x, prev.y + 0.5 * straight.y);
            let mut inserted: Option<Point2D> = None;
            for _try in 0..3 {
                // `rand() & 15` — pick a random 16-sector direction
                // and scale by another 0..15 random magnitude.
                let dir_sector = rng.u32(0..16) as i16;
                let magnitude = rng.u32(0..16) as f32;
                let (dx, dy) = crate::element_kinds::direction_vector_16(dir_sector);
                let scale = magnitude * max_norm * DRUNKEN_DEVIATION_FACTOR * factor;
                let candidate = pt(midpoint.x + dx * scale, midpoint.y + dy * scale);
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
    entities: &'a [Option<Entity>],
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

        let Some(Some(entity)) = self.entities.get(element.0 as usize) else {
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
        let entity_id = EntityId(element.0);
        matches!(
            self.sequence_manager
                .current_order_for_actor(entity_id)
                .map(|(_, _, o)| o.order_type),
            Some(OrderType::BeingWeakSword | OrderType::BeingStunnedSword)
        )
    }

    fn is_unconscious_and_alive(&self, element: crate::titbit::ElementHandle) -> bool {
        let Some(Some(entity)) = self.entities.get(element.0 as usize) else {
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
        self.follow_element.is_some_and(|id| id.0 == element.0)
    }

    fn is_hidden_posture(&self, element: crate::titbit::ElementHandle) -> bool {
        use crate::element::Posture;
        let Some(Some(entity)) = self.entities.get(element.0 as usize) else {
            return false;
        };
        matches!(
            entity.element_data().posture,
            Posture::Spy | Posture::Tree | Posture::AnonymousArcher
        )
    }

    fn random_u32(&self) -> u32 {
        crate::sim_rng::u32(..)
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
        let assets = LevelAssets::new();
        let pc_id = engine.add_entity(make_aiming_pc(action_state));
        engine.launch_element(SequenceElement::new(1, command, Some(pc_id)));

        let mut display = HostDisplayState::default();
        let mut dev = DevState::default();
        engine.perform_hourglass(&mut display, &assets, &mut dev);
        engine
    }

    fn command_order_types(engine: &EngineInner) -> Vec<OrderType> {
        engine
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

    #[test]
    fn bow_lean_out_commands_keep_transition_order_live() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
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
        engine.perform_hourglass(&mut display, &assets, &mut dev);

        let elem = engine.sequence_manager.get_element(seq_id, 0).unwrap();
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
    fn equip_bow_down_terminates_when_actor_is_already_aiming_up() {
        let engine =
            launch_bow_command_and_tick(Command::EquipBowDown, ActionState::AimingWithBowUp);
        let elem = engine
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
}

#[cfg(test)]
mod soldier_take_drink_parity_tests {
    use super::*;
    use crate::element::{
        ActorData, ActorSoldier, ElementBonus, ElementData, ElementKind, ElementProjectile,
        HumanData, NpcData, ObjectData, ObjectType, Point2D as ElementPoint2D, Point3D, Posture,
        ProjectileData, SoldierData,
    };
    use crate::sequence::SequenceElement;

    fn make_soldier_at(x: f32, y: f32) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position(Point3D { x, y, z: 0.0 });
        element.set_position_map(ElementPoint2D { x, y });
        element.set_direction_instantly(0);
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
        element.set_position(Point3D { x, y, z: 0.0 });
        element.set_position_map(ElementPoint2D { x, y });
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
        element.set_position(Point3D { x, y, z: 0.0 });
        element.set_position_map(ElementPoint2D { x, y });
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
        let assets = LevelAssets::new();
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
}

#[cfg(test)]
mod drop_ammo_merge_tests {
    use super::*;
    use crate::campaign::{Campaign, PcDescription};
    use crate::element::{ActorPc, ElementData, ElementKind, EntityId, Posture};
    use crate::geo2d::BBox2D;
    use crate::profiles::{Action, CharacterProfileIdx};
    use crate::sequence::{Field, FieldValue, SequenceElement};

    fn count_bonuses(engine: &EngineInner, action: Action) -> Vec<(EntityId, u16)> {
        engine
            .entities
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| match slot {
                Some(crate::element::Entity::Bonus(b))
                    if b.element.active && b.object.associated_action == action =>
                {
                    Some((EntityId(idx as u32), b.object.quantity))
                }
                _ => None,
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
        engine.campaign = Some(campaign);

        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position_map(crate::element::Point2D { x: 100.0, y: 100.0 });
        element.set_direction_instantly(0);
        // Seed a non-empty move box so try_get_drop_position's
        // is_somewhere check passes.  The exact dims don't matter on
        // an empty grid.
        element
            .sprite
            .position_iface
            .set_move_box(BBox2D::from_corners(
                crate::geo2d::pt(-5.0, -5.0),
                crate::geo2d::pt(5.0, 5.0),
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
        if let Some(Some(entity)) = engine.entities.get_mut(pc_id.0 as usize) {
            entity
                .element_data_mut()
                .set_position_map(crate::element::Point2D { x: 200.0, y: 200.0 });
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
