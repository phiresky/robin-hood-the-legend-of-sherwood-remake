//! Friendly (civilian) AI.
//!
//! This module contains the `FriendlyAi` struct which extends [`AiController`]
//! with civilian-specific state: talking, panic behavior, beggar interactions,
//! and the civilian Think state machine.

use serde::{Deserialize, Serialize};

use crate::ai::*;
use crate::parameters_ai::{
    AB_DELTA_DEFAULT_LOOK_TIME, AB_MIN_DEFAULT_LOOK_TIME, AI_FIRST_LOOK_TIME,
    AI_STANDARD_PANIC_RUNS, AI_TALK_DISTANCE,
};

// ---------------------------------------------------------------------------
// Civilian-specific constants
// ---------------------------------------------------------------------------

pub const APPLE_CHASE_IDEAL_DISTANCE: i32 = 300;
pub const BEGGAR_NO_RANDOM_TALK_DISTANCE: i32 = 100;

// ---------------------------------------------------------------------------
// FriendlyAi — extends AiController with civilian-specific state
// ---------------------------------------------------------------------------

/// Civilian AI state. Extends [`AiController`] with civilian-specific fields.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct FriendlyAi {
    /// Base AI controller (contains all common state).
    pub base: AiController,

    // -- Civilian-specific private fields --
    pub beggar_dont_talk_counter: u16,
    pub fleeing_seen_enemy_counter: u16,
}

impl Default for FriendlyAi {
    fn default() -> Self {
        Self {
            base: AiController {
                current_state: AiState::Default,
                attitude: Attitude::Suspicious,
                ..AiController::default()
            },
            beggar_dont_talk_counter: 0,
            fleeing_seen_enemy_counter: 0,
        }
    }
}

impl FriendlyAi {
    pub fn new(owner: NpcHandle) -> Self {
        Self {
            base: AiController::new(owner),
            ..Default::default()
        }
    }

    // -- Accessors --

    pub fn set_beggar_dont_talk_counter(&mut self, value: u16) {
        self.beggar_dont_talk_counter = value;
    }

    // -- State management --

    /// Set state and substate, update alert status for civilians.
    ///
    /// Unlike the base-class version, this also sets alert status
    /// (green for default/wondering, yellow for seeking/fleeing) and
    /// notifies the script system.
    pub fn set_state(&mut self, state: AiState, substate: Substate) {
        self.base
            .register_log_line(LogLineType::ChangeState, substate as u16);

        // Set alert status based on state (civilians only have green/yellow)
        match state {
            AiState::Sleeping | AiState::Default | AiState::Wondering => {
                self.base.set_alert_status(AlertLevel::Green);
            }
            AiState::Seeking | AiState::Fleeing => {
                self.base.set_alert_status(AlertLevel::Yellow);
            }
            _ => {
                // Civilians should never be in Attacking or Menacing
                panic!("Civilian AI entered invalid state: {:?}", state);
            }
        }

        // Fire an `AI_STATE_CHANGE_TO_*` filter event on every
        // `set_state`.  The civilian gate is just "actor is scripted
        // and scripting is enabled" — no substate check — so every
        // call queues a notification and the engine's dispatcher
        // gates on the actor being scripted at drain time.  Source =
        // primary target for Fleeing, otherwise self; civilians
        // never reach Attacking/Menacing.
        let source = match state {
            AiState::Fleeing => AiStateChangeSource::from_optional_human(self.base.primary_target),
            _ => AiStateChangeSource::SelfActor,
        };
        self.base
            .pending_state_change_notifications
            .push((state, source));

        self.base.set_ai_state(state);
        self.base.current_substate = substate;
    }

    // -- Movement helpers (Shape 1 — see `ai_enemy.rs` section comment) --

    /// Transition to `(state, substate)` and queue a movement to `destination`.
    pub fn go_to(
        &mut self,
        state: AiState,
        substate: Substate,
        destination: Position,
        flags: crate::ai::GotoFlags,
        ctx: &AiContext,
    ) {
        self.set_state(state, substate);
        self.base.go_to(destination, flags, ctx);
    }

    /// Like [`FriendlyAi::go_to`] but with a speed modifier.
    pub fn go_to_speed(
        &mut self,
        state: AiState,
        substate: Substate,
        destination: Position,
        flags: crate::ai::GotoFlags,
        speed: f32,
        ctx: &AiContext,
    ) {
        self.set_state(state, substate);
        self.base.go_to_speed(destination, flags, speed, ctx);
    }

    /// Transition to `(state, substate)` and queue a "go near" movement.
    pub fn go_near(
        &mut self,
        state: AiState,
        substate: Substate,
        destination: Position,
        distance: i32,
        flags: crate::ai::GotoFlags,
        ctx: &AiContext,
    ) {
        self.set_state(state, substate);
        self.base.go_near(destination, distance, flags, ctx);
    }

    // -- Panic helpers (civilians go through set_state for alert status) --
    //
    // Each helper stashes a [`PanicRequest`] on
    // [`AiController::pending_begin_panic`] so the engine can perform
    // the door lookup against `ai_global.door_seek_infos` at
    // post-think time.  The reference flow is synchronous:
    // `GetNearestDoor` → `SetState(FLEEING_RUN_TO_DOOR)` → `GoTo(door)`.
    // The default `FleeingPanic` transition here is the fallback the
    // engine layer will override to `FleeingRunToDoor` on a
    // successful door lookup.

    /// Panic fleeing from a specific point, tagged with the sector
    /// and level of its origin so the engine's door lookup can
    /// resolve multi-level flee paths correctly.
    fn panic_from_point_at(&mut self, center: Position, runs: u8) {
        // Capture the "new panic" flag before the state transition:
        // the drain's no-door arm uses this to suppress repeated
        // SetState / Say / reach-point self-fires when we're already
        // in panic.
        let was_already_fleeing = matches!(
            self.base.current_substate,
            Substate::FleeingPanic | Substate::FleeingRunToDoor
        );
        self.base.panic_center_x = center.x;
        self.base.panic_center_y = center.y;
        self.base.lasting_panic_runs = runs;
        self.base.directed_panic = true;
        self.set_state(AiState::Fleeing, Substate::FleeingPanic);
        self.base.pending_begin_panic = Some(PanicRequest {
            center: Some(center),
            runs,
            alert: AlertLevel::Red,
            is_new_panic: !was_already_fleeing,
        });
    }

    /// Raw-coordinate panic entry point (tests only).  Production
    /// code should use [`Self::panic_from_point_at`] so the panic
    /// center carries a valid sector/level for the multi-level
    /// door lookup.
    #[cfg(test)]
    fn panic_from_point(&mut self, center_x: f32, center_y: f32, runs: u8) {
        self.panic_from_point_at(
            Position {
                x: center_x,
                y: center_y,
                sector: None,
                level: 0,
            },
            runs,
        );
    }

    /// Panic from a position, preserving sector/level.
    fn panic_from_position(&mut self, pos: Position, runs: u8, _ctx: &AiContext) {
        self.panic_from_point_at(pos, runs);
    }

    /// Undirected panic.
    fn panic_undirected(&mut self, runs: u8, _ctx: &AiContext) {
        let was_already_fleeing = matches!(
            self.base.current_substate,
            Substate::FleeingPanic | Substate::FleeingRunToDoor
        );
        self.base.lasting_panic_runs = runs;
        self.base.directed_panic = false;
        self.set_state(AiState::Fleeing, Substate::FleeingPanic);
        self.base.pending_begin_panic = Some(PanicRequest {
            center: None,
            runs,
            alert: AlertLevel::Red,
            is_new_panic: !was_already_fleeing,
        });
    }

    // -----------------------------------------------------------------------
    // Think — main stimulus dispatcher
    // -----------------------------------------------------------------------

    /// Main entry point for civilian stimulus processing.
    pub fn think(
        &mut self,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        doors: Option<&[crate::gate::Door]>,
    ) -> bool {
        self.base.cached_frame = ctx.frame;
        self.base.cached_in_building = ctx.in_building;

        let stimulus_type = stimulus.stimulus_type;

        self.base
            .register_log_line(LogLineType::Event, stimulus_type as u16);

        // Pre-think checks
        if !self.start_think(stimulus, ctx) {
            self.end_think(global, ctx, tick, grid, doors);
            return true;
        }

        // Script filter gate applied by the engine before this call —
        // see `Engine::filter_stimulus` and the matching note in
        // ai_enemy::think.

        self.update_new_task_priority(stimulus);

        let return_value = match stimulus_type {
            // Expected events
            StimulusType::EventReachPoint
            | StimulusType::EventDone
            | StimulusType::EventTimer
            | StimulusType::CallYourTalk1
            | StimulusType::CallYourTalk2
            | StimulusType::CallYourTalk3
            | StimulusType::EventMyTalk1
            | StimulusType::EventMyTalk2
            | StimulusType::EventMyTalk3 => {
                self.think_expected_event(stimulus, global, ctx, tick, grid, doors)
            }

            // Unexpected events
            StimulusType::EventOutOfView
            | StimulusType::EventCouldntReachPoint
            | StimulusType::EventFitAgain
            | StimulusType::EventAfterScriptGoOn
            | StimulusType::EventSeesSoldier
            | StimulusType::CallPatrolCoordinate
            | StimulusType::CallYouJustWait
            | StimulusType::EventAppleChaseNear
            | StimulusType::EventNetAway => self.think_unexpected_event(stimulus, ctx, grid, doors),

            // Alerting events
            StimulusType::EventView
            | StimulusType::EventHear
            | StimulusType::EventPcShotAtMe
            | StimulusType::EventSeesBody
            | StimulusType::EventSeesObject
            | StimulusType::EventSeesFriendInTrouble
            | StimulusType::EventGotHit
            | StimulusType::EventLoseConsciousness
            | StimulusType::EventGetArrow
            | StimulusType::EventPanic
            | StimulusType::EventStop => self.think_alerting_event(stimulus, ctx, grid, doors),

            // Events not handled for civilians.  The original
            // shipping build silently drops the stimulus and returns
            // false; we additionally warn so a misroute is still
            // visible.
            StimulusType::EventObjectAway
            | StimulusType::EventMissesCharly
            | StimulusType::EventSeesCharly
            | StimulusType::EventSyncCharly => {
                tracing::warn!(
                    "FriendlyAi::think: stimulus {:?} not handled for civilians (stale routing?)",
                    stimulus_type
                );
                false
            }

            StimulusType::EventReturnToDuty => {
                // EVENT_RETURN_TO_DUTY runs the duty hand-off but
                // Think returns false.
                self.return_to_duty(DutyFlags::empty(), ctx);
                false
            }

            // Shadows are ignored by civilians; Think returns false.
            StimulusType::EventSeesShadow => false,

            // Unknown stimuli silently no-op with a return of false.
            _ => {
                tracing::warn!(
                    "FriendlyAi::think: unknown stimulus type {:?}",
                    stimulus_type
                );
                false
            }
        };

        self.end_think(global, ctx, tick, grid, doors);
        return_value
    }

    // -----------------------------------------------------------------------
    // Think sub-methods
    // -----------------------------------------------------------------------

    fn start_think(&mut self, stimulus: &Stimulus, ctx: &AiContext) -> bool {
        // Civilian pre-think pipeline.  Civilians normally never
        // hit `EventWasp` / `EventNet`, but the gates live on the
        // base class so any scripted `SetSubstate` could reach them;
        // mirror the enemy path's defensive refusals.
        let stimulus_type = stimulus.stimulus_type;

        self.base.couldnt_reachpoint = false;
        self.base.already_on_point = false;
        self.base.already_turned = false;
        self.base.think_recursion_depth = self.base.think_recursion_depth.saturating_add(1);

        if let StimulusInfo::Human(h) = stimulus.info {
            self.base.last_stimulus_actor = h;
        }

        // LOSE_CONSCIOUSNESS always drops the alert regardless of the
        // downstream refusal — even when the event is otherwise
        // filtered out.
        if stimulus_type == StimulusType::EventLoseConsciousness {
            self.base.set_alert_status(AlertLevel::Green);
        }

        // Freeze gate.
        if self.base.locks_flag_field.contains(AiLockFlags::FREEZE) {
            self.base.register_log_line(LogLineType::EventRefused, 1);
            return false;
        }

        // Script lock — queue non-gameflow stimuli when
        // `remember_events` is set so the script can drain them later.
        if self.base.script_locked {
            if self.base.remember_events {
                match stimulus_type {
                    StimulusType::EventDone | StimulusType::EventReachPoint => {
                        // Gameflow commands — ignore.
                    }
                    _ => {
                        self.base.stimulus_queue.push(*stimulus);
                    }
                }
            }
            self.base.register_log_line(LogLineType::EventRefused, 2);
            return false;
        }

        // Non-FREEZE AI locks (BUSY, BEGGAR) — queue for later.
        let non_freeze = self.base.locks_flag_field - AiLockFlags::FREEZE;
        if !non_freeze.is_empty() {
            self.base.stimulus_queue.push(*stimulus);
            self.base.register_log_line(LogLineType::EventRefused, 3);
            return false;
        }

        // WonderingWaspInArmour gate.
        if self.base.current_substate == Substate::WonderingWaspInArmour {
            match stimulus_type {
                StimulusType::EventLoseConsciousness | StimulusType::EventWaspAway => {}
                _ => {
                    self.base.register_log_line(LogLineType::EventRefused, 4);
                    return false;
                }
            }
        }

        // WonderingUnderNet gate.
        if self.base.current_substate == Substate::WonderingUnderNet {
            match stimulus_type {
                StimulusType::EventLoseConsciousness | StimulusType::EventNetAway => {}
                _ => {
                    self.base.register_log_line(LogLineType::EventRefused, 5);
                    return false;
                }
            }
        }

        // FleeingMerryManLeaveMap gate.  Reached by civilian
        // merry-men running off the map after rescue, so this gate
        // is civilian-relevant.
        if self.base.current_substate == Substate::FleeingMerryManLeaveMap
            && stimulus_type != StimulusType::EventReachPoint
        {
            self.base.register_log_line(LogLineType::EventRefused, 6);
            return false;
        }

        // Reset standing-around timer.
        self.base.standing_around_timer = 0;

        // Stale-timer handling.
        if self.base.timer_is_running {
            if self.base.current_substate != self.base.substate_at_last_timer_launch {
                self.base.timer_is_running = false;
            }
        } else if stimulus_type == StimulusType::EventTimer
            && self.base.current_substate != self.base.substate_at_last_timer_launch
        {
            self.base.register_log_line(LogLineType::EventRefused, 9);
            return false;
        }

        // Dead guys ignore everything.  Defence-in-depth — scripts
        // and cross-NPC actions can still fire stimuli at a corpse
        // even though the tick loop normally skips them.
        if ctx.self_is_dead {
            self.base.register_log_line(LogLineType::EventRefused, 10);
            return false;
        }

        // SleepingUnconscious refusal for non-FitAgain stimuli.
        if self.base.current_substate == Substate::SleepingUnconscious
            && stimulus_type != StimulusType::EventFitAgain
        {
            self.base.register_log_line(LogLineType::EventRefused, 11);
            return false;
        }

        // FitAgain only valid when unconscious or napping; refused
        // even when unconscious if the actor is being carried.
        if stimulus_type == StimulusType::EventFitAgain {
            match self.base.current_substate {
                Substate::SleepingUnconscious | Substate::SleepingNapping => {}
                _ => {
                    self.base.register_log_line(LogLineType::EventRefused, 12);
                    return false;
                }
            }
            if ctx.posture == crate::element::Posture::Carried {
                self.base.register_log_line(LogLineType::EventRefused, 7);
                return false;
            }
        }

        true
    }

    fn end_think(
        &mut self,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        _tick: &AiPerTickData,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
        _doors: Option<&[crate::gate::Door]>,
    ) {
        // legacy implementation EndThink calls Think(EVENT_*) here, and Think runs the
        // script FilterAIEvent gate before dispatch. Queue these as
        // same-frame self-stimuli so the engine-side drain can apply
        // that filter without re-entering the script VM through this
        // borrowed AI object. The three-tier depth gate still matches
        // legacy implementation: <100 queues the follow-up, 100..=110 bails to
        // `ReturnToDuty`, 111+ drops it silently.

        if self.base.couldnt_reachpoint {
            self.base.couldnt_reachpoint = false;
            if self.base.think_recursion_depth < 100 {
                self.base
                    .pending_self_stimuli
                    .push(StimulusType::EventCouldntReachPoint);
            } else if self.base.think_recursion_depth < 111 {
                self.return_to_duty(DutyFlags::empty(), ctx);
            }
        }
        if self.base.already_on_point {
            self.base.already_on_point = false;
            if self.base.think_recursion_depth < 100 {
                self.base
                    .pending_self_stimuli
                    .push(StimulusType::EventReachPoint);
            } else if self.base.think_recursion_depth < 111 {
                self.return_to_duty(DutyFlags::empty(), ctx);
            }
        }
        if self.base.already_turned {
            self.base.already_turned = false;
            if self.base.think_recursion_depth < 100 {
                self.base.pending_self_stimuli.push(StimulusType::EventDone);
            } else if self.base.think_recursion_depth < 111 {
                self.return_to_duty(DutyFlags::empty(), ctx);
            }
        }
        self.base.think_recursion_depth = self.base.think_recursion_depth.saturating_sub(1);
    }

    /// Civilian-side new-task-priority hook is intentionally empty.
    fn update_new_task_priority(&mut self, _stimulus: &Stimulus) {
        // Intentionally empty.
    }

    // -----------------------------------------------------------------------
    // ThinkExpectedEvent — civilian dispatcher
    // -----------------------------------------------------------------------

    fn think_expected_event(
        &mut self,
        stimulus: &Stimulus,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        doors: Option<&[crate::gate::Door]>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;

        match self.base.current_substate {
            // -------- Common stuff for soldiers and civilians --------
            Substate::FleeingPanic => {
                if self.base.lasting_panic_runs == 0 {
                    self.fleeing_seen_enemy_counter = 0;
                }
                // Falls through to the common-stuff dispatcher.
                return self.base.think_expected_event_common_stuff(stimulus, ctx);
            }

            Substate::DefaultGotoPost
            | Substate::DefaultGotoPostTurn
            | Substate::DefaultGotoRoute
            | Substate::DefaultGotoRouteTurn
            | Substate::DefaultOnPost
            | Substate::DefaultEnroute
            | Substate::DefaultInMacro
            | Substate::DefaultInMacroWaitingForDone
            | Substate::FleeingRunToHide
            | Substate::FleeingRunToDoor
            | Substate::FleeingHiding => {
                return self.base.think_expected_event_common_stuff(stimulus, ctx);
            }

            Substate::DefaultHomeSweetHome => {
                // NOP — stay home
            }

            Substate::DefaultPatrolEnroute | Substate::DefaultPatrolEnrouteRunning => {
                if stimulus_type == StimulusType::EventReachPoint {
                    // Only face the patrol direction when the current
                    // facing differs from the assigned one, otherwise
                    // we're already lined up and a no-op turn would
                    // re-trigger animation events.
                    if self.base.patrol_direction != 0
                        && self.base.patrol_direction != ctx.direction
                    {
                        self.base.face_direction(self.base.patrol_direction, ctx);
                    }
                    self.set_state(AiState::Default, Substate::DefaultPatrolEnrouteWaiting);
                }
            }

            Substate::DefaultPatrolEnrouteWaiting => {
                if stimulus_type == StimulusType::EventTimer {
                    // If the patrol chief is still in Default or
                    // Wondering we re-arm the 200-frame waiting
                    // timer; otherwise the chief is in trouble and
                    // we abandon patrol via `ReturnToDuty`.  The
                    // engine caches the chief's AI state on
                    // `tick.patrol_chief_state` each frame so we
                    // don't need a second entity borrow.
                    match tick.patrol_chief_state {
                        AiState::Default | AiState::Wondering => {
                            self.base.launch_timer(200, ctx.frame);
                        }
                        _ => {
                            self.return_to_duty(DutyFlags::empty(), ctx);
                        }
                    }
                }
            }

            Substate::DefaultChildApproachedWhistling => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(DutyFlags::empty(), ctx);
                }
            }

            // ############## W O N D E R I N G #####################
            Substate::WonderingCivilianAdmiringHero => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(DutyFlags::empty(), ctx);
                }
            }

            Substate::WonderingCivilianEnemyReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    let seek_pos = self.base.seek_position;
                    if !self.alert_soldier(seek_pos, 0, ctx, grid, doors) {
                        self.base.say(Remark::CivPanic);
                        let pos = self.base.seek_position;
                        self.panic_from_position(pos, AI_STANDARD_PANIC_RUNS as u8, ctx);
                    }
                }
            }

            Substate::WonderingCivilianBodyReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    let seek_pos = self.base.seek_position;
                    if !self.alert_soldier(seek_pos, 0, ctx, grid, doors) {
                        self.base.say(Remark::CivPanic);
                        let pos = self.base.seek_position;
                        self.panic_from_position(pos, AI_STANDARD_PANIC_RUNS as u8, ctx);
                    }
                }
            }

            Substate::WonderingWatchingWhistling => {
                if stimulus_type == StimulusType::EventTimer {
                    self.base.say(Remark::CivWhistling);
                    let seek_pos = self.base.seek_position;
                    self.go_near(
                        AiState::Wondering,
                        Substate::WonderingChildApproachingWhistling,
                        seek_pos,
                        50,
                        GotoFlags::RUN,
                        ctx,
                    );
                }
            }

            Substate::WonderingChildApproachingWhistling => {
                if stimulus_type == StimulusType::EventReachPoint {
                    self.set_state(AiState::Default, Substate::DefaultChildApproachedWhistling);
                    self.base.launch_timer(100, ctx.frame);
                }
            }

            // ############## S E E K I N G #####################

            // -------- civilian alerts soldier: running to soldier --------
            Substate::SeekingCivilianRunningToSoldier => {
                if stimulus_type == StimulusType::EventReachPoint {
                    let antagonist_handle = self.base.antagonist;
                    let antagonist_view = ctx.entity_view(antagonist_handle);
                    match antagonist_view.map(|v| v.ai_state) {
                        Some(AiState::Default) => {
                            // "You have not seen the officer!" — the
                            // soldier is still on duty, so close the
                            // last few steps and talk to them.  The
                            // outer match arm already proves the
                            // view is `Some(…)` here, so the unwrap
                            // is infallible.
                            let antag_view = antagonist_view
                                .expect("antagonist view resolved in outer match arm");
                            let antag_pos = antag_view.position;
                            let dx = antag_pos.x - ctx.position.x;
                            let dy = antag_pos.y - ctx.position.y;
                            let sq_norm = dx * dx + dy * dy;
                            let talk_sq = (AI_TALK_DISTANCE as f32) * (AI_TALK_DISTANCE as f32);
                            if sq_norm > talk_sq {
                                // Still too far — walk up to the
                                // officer using their forecasted
                                // destination, so the civilian heads
                                // to where they'll be rather than
                                // where they currently animate
                                // (matters when the officer is mid-
                                // door-pass / on a lift / mid-
                                // building traversal).  The 20-frame
                                // re-evaluation timer in the
                                // SeekingCivilianRunningToSoldierSeen
                                // arm catches up if the prediction
                                // was wrong.
                                self.base.go_near(
                                    antag_view.forecasted_destination,
                                    AI_TALK_DISTANCE,
                                    GotoFlags::RUN,
                                    ctx,
                                );
                            } else {
                                // Deliver the alert to the officer
                                // via the deferred inter-NPC Think
                                // queue.  We already verified the
                                // officer is in `STATE_DEFAULT` in
                                // the outer match arm, so the
                                // soldier's `CALL_ALERT` handler
                                // will accept and transition to
                                // `SEEKING_WAIT_FOR_ALERTING_CIVILIAN`.
                                //
                                // ACCEPTED DIVERGENCE: the reference
                                // runs `Think(CALL_ALERT)` on the
                                // soldier synchronously, then
                                // synchronously re-enters its own
                                // `Think(EVENT_REACHPOINT)` against
                                // the new substate so the alert
                                // hand-off completes in one tick.
                                // Rust queues the `CallAlert` cross-
                                // NPC action (drained globally in
                                // `process_pending_cross_npc_actions`,
                                // which runs *after* all per-NPC
                                // `dispatch_think_with_drain` passes)
                                // and pushes `EventReachPoint` onto
                                // `pending_self_stimuli`.  The self-
                                // stimulus drain inside this NPC's
                                // own `dispatch_think_with_drain`
                                // then re-fires `EventReachPoint`
                                // *before* the soldier has processed
                                // `CallAlert`, so the new substate's
                                // handler reads a stale
                                // `entity_view(antagonist)` (still
                                // `STATE_DEFAULT`, not
                                // `SeekingWaitForAlertingCivilian`)
                                // and falls through to
                                // `ReturnToDuty`.  Practical
                                // symptom: when the civilian happens
                                // to start within `AI_TALK_DISTANCE`
                                // of the officer (close branch, not
                                // post-`go_near`), the alert
                                // silently drops and the civilian
                                // wanders off.
                                //
                                // Fixing this end-to-end requires a
                                // synchronous CallAlert dispatch +
                                // mid-think `refresh_ai_entity_views`
                                // helper (~200 LOC, new re-entrancy
                                // guard, new flag).  The cosmetic
                                // blast radius (rare path,
                                // indistinguishable from civilian
                                // RTD wander) doesn't justify the
                                // engine plumbing standalone.
                                // Revisit if cross-NPC drain
                                // ordering is ever refactored —
                                // folding in costs ~50 LOC then.
                                // See parity-audit divergence (2).
                                self.base.pending_cross_npc_actions.push(
                                    CrossNpcAction::SendStimulus {
                                        target: antagonist_handle,
                                        stimulus_type: StimulusType::CallAlert,
                                        info: StimulusInfo::Human(self.base.me),
                                        fallback_to_sender: None,
                                        to_whole_patrol: false,
                                    },
                                );
                                self.base
                                    .pending_delete_detectables
                                    .push(crate::element::DetectableType::Friend);
                                self.set_state(
                                    AiState::Seeking,
                                    Substate::SeekingCivilianRunningToSoldierSeen,
                                );
                                self.base
                                    .pending_self_stimuli
                                    .push(StimulusType::EventReachPoint);
                            }
                        }
                        _ => {
                            // Officer is no longer in STATE_DEFAULT
                            // (reassigned / knocked out / script
                            // interrupted / entity gone) — look for
                            // another soldier and, on failure, fall
                            // back to `ReturnToDuty`.  This also
                            // covers the "antagonist view not in the
                            // entity map" case (entity removed this
                            // tick): the original would crash on a
                            // null antagonist, but the "look for
                            // another officer" branch is the closest
                            // legal analogue since the antagonist
                            // isn't usable either way.
                            let seek_pos = self.base.seek_position;
                            if !self.alert_soldier(seek_pos, 0, ctx, grid, doors) {
                                self.return_to_duty(DutyFlags::empty(), ctx);
                            }
                        }
                    }
                }
            }

            Substate::SeekingCivilianRunningToSoldierSeen => {
                let antag_substate = ctx.entity_view(self.base.antagonist).map(|v| v.ai_substate);
                let waiting = antag_substate == Some(Substate::SeekingWaitForAlertingCivilian);
                match stimulus_type {
                    StimulusType::EventTimer => {
                        if waiting {
                            // Officer is still waiting — re-arm the
                            // timer so we check again in 20 frames.
                            self.base.launch_timer(20, ctx.frame);
                        } else {
                            // Something went wrong (officer got
                            // reassigned / knocked out / script
                            // interrupted) — forget it.
                            self.return_to_duty(DutyFlags::empty(), ctx);
                        }
                    }
                    StimulusType::EventReachPoint => {
                        if waiting {
                            self.set_state(
                                AiState::Seeking,
                                Substate::SeekingCivilianGiveAlertingReportToSoldierStart,
                            );
                            self.base.launch_timer(10, ctx.frame);
                        } else {
                            self.return_to_duty(DutyFlags::empty(), ctx);
                        }
                    }
                    _ => {}
                }
            }

            Substate::SeekingCivilianGiveAlertingReportToSoldierStart => {
                if stimulus_type == StimulusType::EventTimer {
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingCivilianGiveAlertingReportToSoldierPoint,
                    );
                    // Hand the officer our recon report via the
                    // deferred inter-NPC Think queue.  We pass a
                    // Hint carrying our seek point so the soldier's
                    // CALL_REPORT handler can update its own report
                    // without needing to reach back into the
                    // civilian's AI state.  The return value is
                    // ignored — it's fire-and-forget.
                    self.base
                        .pending_cross_npc_actions
                        .push(CrossNpcAction::SendStimulus {
                            target: self.base.antagonist,
                            stimulus_type: StimulusType::CallReport,
                            info: StimulusInfo::Hint(Hint {
                                seek_point: self.base.seek_position,
                                seek_flags: 0,
                                who_tells_me: self.base.me,
                            }),
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                        });
                    self.base.say(Remark::CivDenunciates);
                    let seek_pos = self.base.seek_position;
                    self.base.point_to(seek_pos);
                }
            }

            Substate::SeekingCivilianGiveAlertingReportToSoldierPoint => {
                if stimulus_type == StimulusType::EventDone {
                    self.set_state(
                        AiState::Seeking,
                        Substate::SeekingCivilianGiveAlertingReportToSoldierEnd,
                    );
                    let antagonist = self.base.antagonist;
                    self.base.face_entity(antagonist, ctx);
                    self.base.launch_timer(30, ctx.frame);
                }
            }

            Substate::SeekingCivilianGiveAlertingReportToSoldierEnd => {
                if stimulus_type == StimulusType::EventTimer {
                    let pos = self.base.seek_position;
                    self.panic_from_position(pos, AI_STANDARD_PANIC_RUNS as u8, ctx);
                }
            }

            Substate::SeekingGotStopEvent => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(DutyFlags::empty(), ctx);
                }
            }

            // ############## F L E E I N G #####################

            // -------- child chased for apple --------
            Substate::FleeingChildChased => {
                match stimulus_type {
                    StimulusType::CallYourTalk1 => {
                        self.base.say(Remark::CivChildChasedBySoldier);
                    }
                    StimulusType::EventReachPoint => {
                        if let Some(pos_goal) =
                            self.propose_good_apple_chase_flee_destination(ctx, grid)
                        {
                            // If the chaser is still breathing down
                            // our neck (Chebyshev distance < 150),
                            // sprint harder (1.2× vs 1.0×).
                            let speed = if let Some(antag) = ctx.entity_view(self.base.antagonist) {
                                let dx = (antag.position.x - ctx.position.x).abs();
                                let dy = (antag.position.y - ctx.position.y).abs();
                                if dx.max(dy) < 150.0 { 1.2 } else { 1.0 }
                            } else {
                                1.0
                            };
                            self.go_to_speed(
                                self.base.current_state,
                                self.base.current_substate,
                                pos_goal,
                                GotoFlags::RUN | GotoFlags::DONT_STOP,
                                speed,
                                ctx,
                            );

                            // Is the soldier still chasing me?
                            let still_chasing = matches!(
                                ctx.entity_view(self.base.antagonist).map(|v| v.ai_substate),
                                Some(Substate::WonderingAppleChasingChild)
                                    | Some(Substate::WonderingAppleChasingChildWaiting)
                                    | Some(Substate::WonderingAppleChasingChildEnd)
                            );
                            if !still_chasing {
                                // No longer chased — keep fleeing a
                                // bit more and wind it down.
                                self.base.lasting_panic_runs = 1;
                                self.set_state(
                                    AiState::Fleeing,
                                    Substate::FleeingChildChasedSupplementalRuns,
                                );
                            }
                        } else {
                            // Panic centred on the chaser's live
                            // position so the flee direction is
                            // away from them.  A missing entity
                            // view is a real engine bug — panic
                            // rather than silently flee in some
                            // arbitrary direction.
                            let panic_center = ctx
                                .entity_view(self.base.antagonist)
                                .map(|v| v.position)
                                .expect("antagonist entity view missing during apple-chase panic");
                            self.panic_from_position(
                                panic_center,
                                AI_STANDARD_PANIC_RUNS as u8,
                                ctx,
                            );
                        }
                    }
                    _ => {}
                }
            }

            Substate::FleeingChildChasedSupplementalRuns => {
                if stimulus_type == StimulusType::EventReachPoint {
                    if self.base.lasting_panic_runs > 0 {
                        self.base.lasting_panic_runs -= 1;
                        if let Some(pos_goal) =
                            self.propose_good_apple_chase_flee_destination(ctx, grid)
                        {
                            let flags = if self.base.lasting_panic_runs > 0 {
                                GotoFlags::RUN | GotoFlags::DONT_STOP
                            } else {
                                GotoFlags::RUN
                            };
                            self.go_to(
                                self.base.current_state,
                                self.base.current_substate,
                                pos_goal,
                                flags,
                                ctx,
                            );
                        } else {
                            self.set_state(AiState::Fleeing, Substate::FleeingChildChasedEnd);
                            let antagonist = self.base.antagonist;
                            self.base.face_entity(antagonist, ctx);
                            self.base.launch_timer(20, ctx.frame);
                        }
                    } else {
                        self.set_state(AiState::Fleeing, Substate::FleeingChildChasedEnd);
                        let antagonist = self.base.antagonist;
                        self.base.face_entity(antagonist, ctx);
                        self.base.launch_timer(20, ctx.frame);
                    }
                }
            }

            Substate::FleeingChildFriendChased => {
                if stimulus_type == StimulusType::EventReachPoint {
                    let antagonist = self.base.antagonist;
                    self.base.face_entity(antagonist, ctx);
                    self.set_state(AiState::Fleeing, Substate::FleeingChildChasedEnd);
                    self.base.launch_timer(50, ctx.frame);
                }
            }

            Substate::FleeingChildChasedEnd => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(DutyFlags::empty(), ctx);
                }
            }

            Substate::DefaultScriptDriven => {
                // NOP — script handles everything
            }

            _ => {
                tracing::warn!(
                    "FriendlyAi::think_expected_event: unhandled substate {:?} \
                     with stimulus {:?}",
                    self.base.current_substate,
                    stimulus_type,
                );
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // ThinkUnexpectedEvent — civilian dispatcher
    // -----------------------------------------------------------------------

    fn think_unexpected_event(
        &mut self,
        stimulus: &Stimulus,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        // `_doors` is not consumed by the Apple-Chase / Sees-Soldier
        // arms in this dispatcher; threaded for symmetry with the
        // other think_* methods so the caller doesn't need to know
        // which sub-handlers reach `alert_soldier`.
        _doors: Option<&[crate::gate::Door]>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;

        match stimulus_type {
            StimulusType::EventSeesSoldier
                if self.base.current_substate == Substate::SeekingCivilianRunningToSoldier =>
            {
                if let StimulusInfo::Human(soldier_handle) = stimulus.info {
                    self.base.antagonist = soldier_handle;
                    // Clear friend detection list — we've reached the soldier.
                    self.base
                        .pending_delete_detectables
                        .push(crate::element::DetectableType::Friend);

                    // The soldier's CALL_ALERT handler accepts iff
                    // its state is STATE_DEFAULT.  We check up-front
                    // via the per-tick entity view snapshot, then
                    // queue the actual stimulus for deferred
                    // delivery so the soldier's state-machine
                    // transition happens on the cross-NPC action
                    // pass.
                    let alert_accepted = ctx
                        .entity_view(soldier_handle)
                        .is_some_and(|v| v.ai_state == AiState::Default);

                    if alert_accepted {
                        self.base
                            .pending_cross_npc_actions
                            .push(CrossNpcAction::SendStimulus {
                                target: soldier_handle,
                                stimulus_type: StimulusType::CallAlert,
                                info: StimulusInfo::Human(self.base.me),
                                fallback_to_sender: None,
                                to_whole_patrol: false,
                            });
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingCivilianRunningToSoldierSeen,
                        );
                        self.base.say(Remark::CivCallsSoldier);

                        // Run to the forecasted destination of the
                        // newly spotted soldier.  STATE_DEFAULT
                        // covers patrol-walking soldiers (not just
                        // standing-still ones), and those routinely
                        // traverse doors / lifts where the forecast
                        // diverges from the live position.  The 20-
                        // frame re-evaluation timer below catches up
                        // if the prediction misses.
                        let target_pos = ctx
                            .entity_view(soldier_handle)
                            .map(|v| v.forecasted_destination)
                            .expect("soldier entity_view must exist — alert_accepted check resolved it above");
                        self.base
                            .go_near(target_pos, AI_TALK_DISTANCE, GotoFlags::RUN, ctx);
                        self.base.launch_timer(20, ctx.frame);
                    } else {
                        self.panic_undirected(AI_STANDARD_PANIC_RUNS as u8, ctx);
                    }
                }
            }

            StimulusType::CallPatrolCoordinate => {
                self.base
                    .coordinate_patrol(&stimulus.info, ctx, &AiPerTickData::stub());
            }

            StimulusType::EventAfterScriptGoOn => {
                // Drain any stimuli that the script lock held back,
                // queuing them on `pending_self_stimuli` so the
                // engine's self-stimulus drain re-fires them in
                // order.  We defer instead of recursing to avoid
                // re-borrowing the `&mut AiGlobalState` our caller
                // already holds.  This drain runs unconditionally
                // on every `EventAfterScriptGoOn`; the
                // `state==Default` gate is only on the tail branch
                // below.
                //
                // Re-check the AI lock / script-lock flags at the
                // top of every iteration and return false if either
                // becomes set, leaving the remaining queued stimuli
                // for the next `EventAfterScriptGoOn`.  A lock that
                // was already set before this call (e.g. acquired
                // by a different dispatch path that bypassed
                // `start_think`) must leave the queue intact so the
                // next `EventAfterScriptGoOn` after the script
                // unlocks can pick up where this one left off.
                while !self.base.stimulus_queue.is_empty() {
                    if !self.base.locks_flag_field.is_empty() || self.base.script_locked {
                        return false;
                    }
                    let q = self.base.stimulus_queue.remove(0);
                    if q.stimulus_type != StimulusType::EventAfterScriptGoOn {
                        self.base.pending_self_stimuli.push(q.stimulus_type);
                    }
                }

                // After the drain, if we're in STATE_DEFAULT we
                // either advance on the patrol path (next waypoint
                // → SetState(Enroute) → GoTo) or call
                // `ReturnToDuty`.  Outside STATE_DEFAULT we leave
                // the state untouched — a script may have committed
                // a sleeping / fleeing / wondering pose and we must
                // not clobber it.
                if self.base.current_state == AiState::Default {
                    let hiking_paths = &ctx.hiking_paths;
                    let has_waypoint = self
                        .base
                        .patrol_path
                        .as_ref()
                        .and_then(|p| p.current_waypoint(hiking_paths))
                        .is_some();
                    if has_waypoint {
                        // Advance to the next waypoint and walk
                        // onto it with the default walking flags.
                        if let Some(ref mut path) = self.base.patrol_path {
                            path.advance();
                        }
                        let dest_flags = self
                            .base
                            .patrol_path
                            .as_ref()
                            .and_then(|p| p.current_waypoint(hiking_paths))
                            .map(|wp| {
                                (
                                    Position {
                                        x: wp.x as f32,
                                        y: wp.y as f32,
                                        sector: crate::position_interface::SectorHandle::new(
                                            wp.sector,
                                        ),
                                        level: wp.level,
                                    },
                                    self.base.default_path_walking_flags,
                                )
                            });
                        if let Some((dest, flags)) = dest_flags {
                            self.go_to(
                                AiState::Default,
                                Substate::DefaultEnroute,
                                dest,
                                flags,
                                ctx,
                            );
                        } else {
                            self.return_to_duty(DutyFlags::empty(), ctx);
                        }
                    } else {
                        self.return_to_duty(DutyFlags::empty(), ctx);
                    }
                    return false;
                }
            }

            StimulusType::CallYouJustWait => {
                // Soldier tells child to wait (apple chase begins)
                if let StimulusInfo::Human(soldier_handle) = stimulus.info {
                    self.base.antagonist = soldier_handle;

                    if let Some(pos_goal) =
                        self.propose_good_apple_chase_flee_destination(ctx, grid)
                    {
                        self.go_to(
                            AiState::Fleeing,
                            Substate::FleeingChildChased,
                            pos_goal,
                            GotoFlags::RUN,
                            ctx,
                        );
                    } else {
                        // Fire a *directed* panic centred on the
                        // chaser so the flee vector biases away
                        // from them.  If our entity view is missing
                        // we fall through to undirected as the
                        // closest legal analogue (the original
                        // would null-deref if the antagonist is
                        // gone).
                        if let Some(antag) = ctx.entity_view(soldier_handle) {
                            self.panic_from_point_at(antag.position, AI_STANDARD_PANIC_RUNS as u8);
                        } else {
                            self.panic_undirected(AI_STANDARD_PANIC_RUNS as u8, ctx);
                        }
                    }
                }
            }

            StimulusType::EventAppleChaseNear => {
                // Nearby apple chase — friend flees too
                if let StimulusInfo::Human(soldier_handle) = stimulus.info {
                    self.base.antagonist = soldier_handle;

                    if let Some(pos_goal) =
                        self.propose_good_apple_chase_flee_destination(ctx, grid)
                    {
                        self.go_to(
                            AiState::Fleeing,
                            Substate::FleeingChildFriendChased,
                            pos_goal,
                            GotoFlags::RUN,
                            ctx,
                        );
                    } else {
                        // Directed panic from the chaser's live
                        // position, same as the CallYouJustWait
                        // fallback above.
                        if let Some(antag) = ctx.entity_view(soldier_handle) {
                            self.panic_from_point_at(antag.position, AI_STANDARD_PANIC_RUNS as u8);
                        } else {
                            self.panic_undirected(AI_STANDARD_PANIC_RUNS as u8, ctx);
                        }
                    }
                }
            }

            StimulusType::EventCouldntReachPoint => {
                if self.base.current_substate == Substate::FleeingPanic {
                    if self.base.lasting_panic_runs == 0 {
                        self.fleeing_seen_enemy_counter = 0;
                    }
                    self.base.think_expected_event_common_stuff(stimulus, ctx);
                } else {
                    self.return_to_duty(DutyFlags::empty(), ctx);
                }
            }

            StimulusType::EventNetAway => {
                let pos = self.base.seek_position;
                self.panic_from_position(pos, AI_STANDARD_PANIC_RUNS as u8, ctx);
            }

            StimulusType::EventFitAgain => {
                // Recovered from unconsciousness.  Three steps:
                //   1. fan out to every other NPC to delete `this`
                //      from their DETECTABLE_BODY lists,
                //   2. snap our eyes back to "look forward" so
                //      refresh_view doesn't keep us blind,
                //   3. return to duty.
                // The first two run through pending flags the engine
                // drains in post-think (analogous to
                // `pending_inform_my_friends` on the "I went down"
                // side of the KO cycle).
                self.base.pending_inform_resurrection = true;
                self.base.pending_set_eye_status = Some(crate::element::EyeStatus::LookForward);
                self.return_to_duty(DutyFlags::empty(), ctx);
            }

            StimulusType::EventOutOfView => {
                // Lost sight of someone — civilians don't react
            }

            _ => {}
        }

        false
    }

    // -----------------------------------------------------------------------
    // ThinkAlertingEvent — civilian dispatcher
    // -----------------------------------------------------------------------

    fn think_alerting_event(
        &mut self,
        stimulus: &Stimulus,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        doors: Option<&[crate::gate::Door]>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;

        match stimulus_type {
            StimulusType::EventView => {
                if let StimulusInfo::Human(human_handle) = stimulus.info {
                    match self.base.current_state {
                        AiState::Default | AiState::Wondering => {
                            self.event_view_standard_procedure(human_handle, ctx);
                        }
                        AiState::Seeking => {
                            // Only update the recon report when the
                            // spotted human is from a *different*
                            // camp (enemy).  Same camp → noop (it's
                            // a friend); different camp → refresh
                            // the report with the live human
                            // position.  `seek_position` is stale
                            // here — it still holds the previous
                            // encounter's last-seen point — so the
                            // report update uses the currently-
                            // spotted human's position instead.
                            if let Some(view) = ctx.entity_view(human_handle)
                                && view.camp != ctx.camp
                            {
                                self.base
                                    .my_reconnaissance_report
                                    .update(ReportType::Enemy, view.position);
                            }
                        }
                        AiState::Fleeing => {
                            // Gate on either different camp or the
                            // spotted human currently swordfighting.
                            // Look up both flags via the per-tick
                            // view map.
                            let Some(v) = ctx.entity_view(human_handle) else {
                                return false;
                            };
                            let different_camp = v.camp != ctx.camp;
                            let is_swordfighting = v.is_swordfighting;
                            let human_pos = v.position;
                            if (different_camp || is_swordfighting)
                                && (self.base.current_substate == Substate::FleeingHiding
                                    || self.fleeing_seen_enemy_counter < 7)
                            {
                                self.fleeing_seen_enemy_counter += 1;
                                self.base
                                    .say_with_flags(Remark::CivPanic, SpeechFlags::HOUSE);
                                // Fire a *directed* panic, fleeing
                                // away from the spotted human.  The
                                // engine's
                                // `process_pending_begin_panic_for`
                                // reads the panic center to pick a
                                // door on the far side, and the
                                // `FleeingPanic` fallback uses it to
                                // bias the random escape vector.
                                self.panic_from_point_at(human_pos, AI_STANDARD_PANIC_RUNS as u8);
                            }
                        }
                        _ => {
                            panic!(
                                "Civilian in invalid state {:?} during EVENT_VIEW",
                                self.base.current_state
                            );
                        }
                    }
                }
            }

            StimulusType::EventSeesBody => {
                if let StimulusInfo::Human(body_handle) = stimulus.info {
                    match self.base.current_state {
                        AiState::Default | AiState::Wondering => {
                            self.event_sees_body_standard_procedure(body_handle, ctx);
                        }
                        _ => {
                            // Other states: ignore bodies
                        }
                    }
                }
            }

            StimulusType::EventHear => {
                if let StimulusInfo::Noise(noise) = stimulus.info {
                    match self.base.current_state {
                        AiState::Sleeping
                        | AiState::Default
                        | AiState::Wondering
                        | AiState::Seeking => {
                            self.event_hear_standard_procedure(&noise, ctx, grid, doors);
                        }
                        AiState::Menacing | AiState::Fleeing | AiState::Attacking => {
                            // Ignore sounds while fighting/fleeing
                        }
                    }
                }
            }

            StimulusType::EventPanic => {
                if let StimulusInfo::Position(pos) = stimulus.info {
                    // The stimulus position carries sector/level
                    // already; preserve them for the multi-level
                    // door lookup in
                    // `process_pending_begin_panic_for`.
                    self.panic_from_point_at(pos, AI_STANDARD_PANIC_RUNS as u8);
                }
            }

            StimulusType::EventStop => {
                if self.base.current_state == AiState::Sleeping {
                    return false;
                }
                self.base.stop_all();
                self.set_state(AiState::Seeking, Substate::SeekingGotStopEvent);
                self.base.launch_timer(100, ctx.frame);
            }

            // These alerting events are dispatched but not handled
            // by civilians — fall through and return false.
            StimulusType::EventPcShotAtMe
            | StimulusType::EventSeesObject
            | StimulusType::EventSeesFriendInTrouble
            | StimulusType::EventGotHit
            | StimulusType::EventLoseConsciousness
            | StimulusType::EventGetArrow => {}

            _ => {}
        }

        false
    }

    // -----------------------------------------------------------------------
    // Standard procedures
    // -----------------------------------------------------------------------

    /// Return to default duty behavior.
    pub fn return_to_duty(&mut self, flags: DutyFlags, ctx: &AiContext) {
        self.fleeing_seen_enemy_counter = 0;

        // "Very very busy" gates on a posture that can't be
        // interrupted mid-transition: Flying / OnLadder / OnWall,
        // or an active PassDoor / Fall sequence element.  The
        // posture arm is checked off `ctx.posture`; the sequence-
        // element arm arrives via `ctx.in_uninterruptible_command`,
        // populated by `build_ai_context_from_entity` from
        // `EngineInner::is_very_very_busy`'s command-element check
        // (`Command::PassDoor | Command::Fall` for the actor's
        // currently-in-flight sequence element).  Defer the
        // re-entry via `pending_self_stimuli` so the AI re-evaluates
        // once the busy state clears (recursive
        // `Think(EVENT_RETURN_TO_DUTY)` after the lock).
        use crate::element::Posture;
        if ctx.in_uninterruptible_command
            || matches!(
                ctx.posture,
                Posture::Flying | Posture::OnLadder | Posture::OnWall,
            )
        {
            self.base.non_script_lock(AiLockFlags::BUSY);
            self.base.was_busy = true;
            self.base
                .pending_self_stimuli
                .push(StimulusType::EventReturnToDuty);
            return;
        }

        // Call the common return-to-duty method for civilians and villains
        self.base.return_to_duty_common_stuff(flags, ctx);
    }

    /// Standard procedure when a civilian sees a PC.
    pub fn event_view_standard_procedure(&mut self, good_guy: HumanHandle, ctx: &AiContext) {
        // Antagonist info is resolved by the engine before dispatch.
        // Absent (None) means the stimulus's target entity went away —
        // treat as "nothing to react to" and bail.
        let Some(antagonist) = ctx.antagonist.as_ref() else {
            return;
        };

        // First check: is the spotted human swordfighting?  Fire a
        // *directed* panic from their position so the engine's
        // `process_pending_begin_panic_for` can bias the door lookup
        // to the *far* side of the swordfighter, rather than picking
        // one we'd run straight past the fighter to reach.
        if antagonist.is_swordfighting {
            self.panic_from_point_at(antagonist.position, AI_STANDARD_PANIC_RUNS as u8);
            return;
        }

        let same_camp = antagonist.camp == ctx.camp;

        if same_camp {
            match self.base.current_state {
                AiState::Default | AiState::Wondering => {
                    // Wow! A hero!  Only emit the admire reaction
                    // for Robin specifically (PC + IsRobin).
                    self.set_state(AiState::Wondering, Substate::WonderingCivilianAdmiringHero);
                    if antagonist.is_pc && antagonist.is_robin {
                        self.base.say(Remark::CivAdmiresRobin);
                    }
                    self.base.stop_all();
                    self.base.face_entity(good_guy, ctx);
                    self.base.launch_timer(AI_FIRST_LOOK_TIME as u32, ctx.frame);
                }
                _ => {}
            }
        } else {
            // `ctx.in_building` is set from the building sector of
            // the evaluating civilian.
            if ctx.in_building {
                // Inside house — panic!
                self.base
                    .say_with_flags(Remark::CivPanic, SpeechFlags::HOUSE);
                self.panic_undirected(AI_STANDARD_PANIC_RUNS as u8, ctx);
            } else {
                // Outside — reaction time before alerting.
                self.base.primary_target = good_guy;
                self.base.seek_position = antagonist.position;
                self.set_state(
                    AiState::Wondering,
                    Substate::WonderingCivilianEnemyReactiontime,
                );
                self.base.stop_all();
                let seek_pos = self.base.seek_position;
                self.base
                    .my_reconnaissance_report
                    .update(ReportType::Enemy, seek_pos);
                self.base.face_position(seek_pos);
                self.base.launch_timer(30, ctx.frame);
            }
        }
    }

    /// Standard procedure when a civilian hears something.
    pub fn event_hear_standard_procedure(
        &mut self,
        noise: &Noise,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        doors: Option<&[crate::gate::Door]>,
    ) {
        match noise.noise_type {
            // Whistling — only children react.
            NoiseType::Pfiiit if ctx.self_is_child => {
                self.base.set_emoticon(EmoticonType::QuestionMark);
                self.set_state(AiState::Wondering, Substate::WonderingWatchingWhistling);
                self.base.seek_position = noise.origin;
                self.base.face_position(noise.origin);
                self.base.launch_timer(70, ctx.frame);
            }
            NoiseType::Aaargh => {
                // Scream — try to alert a soldier
                self.base.seek_position = noise.origin;

                // On a Royalist civilian's scream, the civilian
                // panics directly instead of alerting a (nearby,
                // also Royalist) soldier.
                let is_royalist = ctx.camp == crate::element::Camp::Royalists;

                if is_royalist || !self.alert_soldier(noise.origin, 0, ctx, grid, doors) {
                    let pos = self.base.seek_position;
                    self.panic_from_position(pos, AI_STANDARD_PANIC_RUNS as u8, ctx);
                }
            }
            _ => {
                // Other noise types — civilians don't react
            }
        }
    }

    /// Standard procedure when a civilian sees a body.
    pub fn event_sees_body_standard_procedure(&mut self, _dead_guy: HumanHandle, ctx: &AiContext) {
        // The engine resolves the body's live position into
        // `ctx.antagonist` before dispatch.
        if let Some(antag) = ctx.antagonist.as_ref() {
            self.base.seek_position = antag.position;
        }
        self.set_state(
            AiState::Wondering,
            Substate::WonderingCivilianBodyReactiontime,
        );
        self.base.stop_all();
        self.base.say(Remark::CivSeesBody);
        let seek_pos = self.base.seek_position;
        self.base
            .my_reconnaissance_report
            .update(ReportType::Body, seek_pos);
        self.base.face_position(seek_pos);
        self.base.launch_timer(AI_FIRST_LOOK_TIME as u32, ctx.frame);
    }

    /// Standard procedure when a civilian sees an object.
    ///
    /// Intentionally empty — no civilian reaction is implemented.
    pub fn event_sees_object_standard_procedure(&mut self, _object: ObjectHandle) {
        // Intentionally empty.
    }

    /// Run to a soldier to report something.
    ///
    /// Declared in the header but never implemented; kept as a stub.
    pub fn run_to_your_big_brother(&mut self, _evil_pc: ElementHandle, _reason: Question) {
        // No implementation exists — treated as dead code.
    }

    /// Hide in a house.
    ///
    /// Declared in the header but never implemented; kept as a stub.
    pub fn hide_in_a_house(&mut self) {
        // No implementation exists — treated as dead code.
    }

    /// Alert a nearby soldier.
    ///
    /// Algorithm:
    /// 1. Walk every able-to-fight, non-script-locked soldier in the
    ///    same camp.  Along the way:
    ///    - Add each candidate to our `DETECTABLE_FRIEND` list (so
    ///      later "is my alerted ally still nearby" checks work).
    ///    - If any of them is in STATE_ATTACKING / STATE_MENACING /
    ///      STATE_FLEEING *and* within our 360° detection radius,
    ///      short-circuit: an alerted soldier is already close by,
    ///      so alerting another one would be noise.
    /// 2. Of the STATE_DEFAULT candidates, pick the MaxNorm-nearest
    ///    with a +1000 layer-change penalty for soldiers on a
    ///    different floor.
    /// 3. When `ALERTFLAG_CHECK_DOOR_PATH` is set *and* we have a
    ///    grid reference, reject candidates whose gate-graph path
    ///    from our sector is unroutable (lifts / locked doors).
    /// 4. Run to the picked soldier and transition to
    ///    SEEKING_CIVILIAN_RUNNING_TO_SOLDIER.
    pub const ALERTFLAG_CHECK_DOOR_PATH: u16 = 0x0001;

    pub fn alert_soldier(
        &mut self,
        center: Position,
        flags: u16,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        doors: Option<&[crate::gate::Door]>,
    ) -> bool {
        let my_pos = ctx.position;
        let my_camp = ctx.camp;
        let my_layer = ctx.position.level;
        let my_sector = ctx.position.sector;
        let check_door_path = (flags & Self::ALERTFLAG_CHECK_DOOR_PATH) != 0;
        let sq_view_radius = ctx.sq_standard_view_radius;
        const OO: u32 = u32::MAX;

        let mut best: Option<(NpcHandle, u32, Position)> = None;
        let mut detectables_to_add: Vec<(
            crate::element::EntityId,
            crate::element::DetectableType,
        )> = Vec::new();

        for (&handle, view) in ctx.entity_views.iter() {
            if handle == self.base.me {
                continue;
            }
            if !view.is_soldier() || view.camp != my_camp {
                continue;
            }
            if !view.is_able_to_fight {
                continue;
            }
            // Skip script-locked soldiers entirely so a
            // `WaitInformatively`-scripted guard isn't dragged off-
            // script by an unrelated civilian alert.
            if view.script_locked {
                continue;
            }

            // On the non-door-path pass we register the soldier as
            // a friend-detectable so the follow-up "someone alerted
            // me" checks later find it.
            if !check_door_path {
                detectables_to_add.push((
                    crate::element::EntityId(handle),
                    crate::element::DetectableType::Friend,
                ));
            }

            match view.ai_state {
                AiState::Default => {
                    // MaxNorm (Chebyshev) distance candidate.
                    let dx = (view.position.x - my_pos.x).abs();
                    let dy = (view.position.y - my_pos.y).abs();
                    let mut distance = dx.max(dy) as u32;

                    // +1000 layer-change penalty.
                    if view.position.level != my_layer {
                        distance = distance.saturating_add(1000);
                    }

                    let prev_best = best.map(|(_, d, _)| d).unwrap_or(OO);

                    // On the door-path retry, perform a gate-graph
                    // reachability check against the door table.
                    // When unreachable, force `distance = OO` so the
                    // candidate loses the MaxNorm comparison.  Needs
                    // `Door` slice + the actor's auth bitmask
                    // (lockpick / climb / jump / posture / kind);
                    // both arrive as parameters.  When `doors` /
                    // `grid` aren't threaded (unit tests), skip the
                    // reachability filter.
                    let unreachable = if check_door_path
                        && let (Some(doors_slice), Some(my_sec), Some(goal_sec)) =
                            (doors, my_sector, view.position.sector)
                        && my_sec != goal_sec
                    {
                        let auth = crate::gate::ActorAuthInfo {
                            kind: crate::element::ElementKind::ActorCivilian,
                            pc_auth_bit: 0,
                            has_lockpick: false,
                            has_climb: false,
                            has_jump: false,
                            is_rider: false,
                            posture: ctx.posture,
                        };
                        crate::gate::find_path_gates(
                            doors_slice,
                            (my_pos.x, my_pos.y),
                            u16::from(my_sec),
                            (view.position.x, view.position.y),
                            u16::from(goal_sec),
                            Some(&auth),
                            false,
                            &|sector| {
                                let grid = grid.unwrap_or_else(|| {
                                    panic!(
                                        "alert_soldier gate path needs grid to resolve lift sector {sector}"
                                    )
                                });
                                grid.level
                                    .sector_number_map
                                    .get(&sector)
                                    .and_then(|&idx| grid.level.sectors.get(idx))
                                    .and_then(|gs| gs.lift_type)
                            },
                        )
                        .is_none()
                    } else {
                        false
                    };
                    let _ = grid;
                    if unreachable {
                        continue;
                    }

                    if distance < prev_best {
                        best = Some((handle, distance, view.forecasted_destination));
                    }
                }
                AiState::Attacking | AiState::Menacing | AiState::Fleeing => {
                    // An alerted soldier is already nearby — no
                    // need to alert another.  `is_detecting_360_*`
                    // is an aspect-ratio-corrected distance within
                    // squared real view radius, the same
                    // approximation `EnemyAi` uses.
                    let dx = view.position.x - my_pos.x;
                    let dy = (view.position.y - my_pos.y)
                        * crate::position_interface::INVERSE_ASPECT_RATIO;
                    if dx * dx + dy * dy <= sq_view_radius {
                        // Clear the friend list and return false.
                        // We queue the clear — the engine drains it
                        // post-think.
                        self.base
                            .pending_delete_detectables
                            .push(crate::element::DetectableType::Friend);
                        return false;
                    }
                }
                _ => {}
            }
        }

        // Queue the friend-detectable adds we accumulated above.
        // Done here (not inline) so the early-return above doesn't
        // add detectables we're about to drop.
        self.base.pending_add_detectables.extend(detectables_to_add);

        let Some((target_handle, _, target_pos)) = best else {
            // No candidate found — clear friend list and give up.
            self.base
                .pending_delete_detectables
                .push(crate::element::DetectableType::Friend);
            return false;
        };

        self.base.antagonist = target_handle;
        self.base.seek_position = center;
        self.set_state(AiState::Seeking, Substate::SeekingCivilianRunningToSoldier);
        // Run toward the picked soldier's forecasted destination
        // (e.g. the far side of an in-flight door pass) rather
        // than the animated mid-traversal position.  `target_pos`
        // is sourced from `view.forecasted_destination`, populated
        // for human actors by `build_entity_views` via
        // `forecast_destination_for_ia`.
        self.base
            .go_near(target_pos, AI_TALK_DISTANCE, GotoFlags::RUN, ctx);

        // On `couldnt_reachpoint`, retry with the door-path flag
        // set so unreachable candidates are filtered out.
        // `couldnt_reachpoint` isn't set synchronously by `go_near`
        // — pathfinding runs asynchronously — so this retry path
        // can only fire if a previous tick's pathfinding already
        // set the flag.  Keep the check for future parity.
        if self.base.couldnt_reachpoint {
            self.base.couldnt_reachpoint = false;
            if !check_door_path {
                return self.alert_soldier(
                    center,
                    Self::ALERTFLAG_CHECK_DOOR_PATH,
                    ctx,
                    grid,
                    doors,
                );
            }
            self.base
                .pending_delete_detectables
                .push(crate::element::DetectableType::Friend);
            return false;
        }

        self.base.say(Remark::CivPanic);
        true
    }

    /// Standard procedure when seeing a friendly soldier (hint opportunity).
    ///
    /// Declared in the header but never implemented; kept as a stub.
    pub fn event_sees_big_brother_standard_procedure(&mut self, _big_brother: NpcHandle) {
        // No implementation exists — treated as dead code.
    }

    /// Random ambient speech for civilians.
    ///
    /// Called each frame; only acts every 256 frames (`frame_phase == 0`).
    pub fn random_speech(&mut self, frame_phase: u8, ctx: &AiContext) {
        if frame_phase != 0 {
            return;
        }

        // ---- executed only every 256 frames ----

        // `ctx.self_is_beggar` is populated by the engine in
        // `build_ai_context_from_entity` from
        // `CivilianData::cached_civilian_type`.
        if ctx.self_is_beggar {
            if self.beggar_dont_talk_counter > 0 {
                self.beggar_dont_talk_counter -= 1;
            } else if self.base.current_remark == Remark::TheSoundOfSilence
                && crate::sim_rng::u32(0..3) == 0
            {
                match crate::sim_rng::u32(0..5) {
                    0..=2 => self.base.say(Remark::CivBeggarBegging),
                    3 => self.base.say(Remark::CivUnderNet),
                    4 => self.base.say(Remark::CivCries),
                    _ => unreachable!(),
                }
            }
        }

        // If our own current animation is Weeping, say "cries".
        // Resolves through the per-tick entity view map —
        // `self.base.me` is the civilian's own handle.
        if let Some(me_view) = ctx.entity_view(self.base.me)
            && me_view.current_animation == crate::order::OrderType::Weeping
        {
            self.base.say(Remark::CivCries);
        }
    }

    /// The 16th-frame hourglass — periodic stuck detection for civilians.
    ///
    /// Called every 16 frames; only acts every 64 frames.
    ///
    /// - `is_idle` is true when the actor's current command is `Wait`.
    /// - `sequence_null_about_to_launch` is set by the caller from
    ///   `sequence_manager.element_is_about_to_be_launched(id,
    ///   Command::Null)`.  When `true`, we skip bumping the stuck
    ///   counter: a queued Null command means the NPC is mid-
    ///   transition (door pass, ladder) and the WAIT state is
    ///   transient, not stuck.
    pub fn the_16th_frame(
        &mut self,
        frame_phase: u8,
        _global: &mut AiGlobalState,
        is_idle: bool,
        sequence_null_about_to_launch: bool,
    ) {
        if (frame_phase & 63) != 0 {
            return;
        }

        // ---- executed only every 64 frames ----

        // Security mechanism against NPCs stuck waiting for EVENT_REACHPOINT.
        match self.base.current_substate {
            Substate::DefaultPatrolEnroute
            | Substate::DefaultPatrolEnrouteRunning
            | Substate::WonderingChildApproachingWhistling
            | Substate::SeekingCivilianRunningToSoldier
            | Substate::SeekingCivilianRunningToSoldierSeen
            | Substate::FleeingChildChased
            | Substate::FleeingChildChasedSupplementalRuns
            | Substate::FleeingChildFriendChased
            | Substate::DefaultGotoPost
            | Substate::DefaultGotoRoute
            | Substate::DefaultEnroute
            | Substate::FleeingRunToHide
            | Substate::FleeingRunToDoor
            | Substate::FleeingPanic => {
                // Whitelisted substate.  Only the idle (Wait)
                // command is acted on — a non-idle command in this
                // substate leaves the counter untouched.
                if is_idle {
                    // Only bump stuck_counter when the sequence
                    // manager is *not* about to launch a Null
                    // command for this actor — a queued Null means
                    // "transition sequence in flight" (door pass,
                    // ladder mount), and re-issuing GoTo now would
                    // collide with the transition.
                    if sequence_null_about_to_launch {
                        self.base.stuck_counter = 0;
                    } else if self.base.stuck_counter < 3 {
                        // Give him some more time.
                        self.base.stuck_counter += 1;
                    } else {
                        // Relaunch and reset.
                        let dest = self.base.last_goto_destination;
                        if dest.x != 0.0 || dest.y != 0.0 {
                            let flags = self.base.last_goto_flags;
                            // Rebuild the order directly; we can't
                            // rebuild an `AiContext` here, so emit
                            // the raw `Order` and let the engine's
                            // drain path issue the path request.
                            let order = AiController::make_move_order(&dest, flags);
                            self.base.pending_orders.push(order);
                        } else {
                            self.base
                                .pending_self_stimuli
                                .push(StimulusType::EventCouldntReachPoint);
                        }
                        self.base.stuck_counter = 0;
                    }
                }
            }
            _ => {
                // Default arm: reset the stuck counter.
                self.base.stuck_counter = 0;
            }
        }
    }

    /// Initialize civilian AI after loading.
    ///
    /// The per-entity wiring (direction/view radius/detectables/
    /// initial position/patrol path creation + fine-check) is
    /// handled by `EngineInner::init_one_ai` before this runs; here
    /// we only handle the beggar-lock + initial-action / return-to-
    /// duty tail.  The returned [`InitStateSideEffects`] carries the
    /// entity-side mutations the caller must apply on NpcData /
    /// HumanData / ElementData / ActorData.
    pub fn init_one_ai(&mut self, ctx: &AiContext) -> InitStateSideEffects {
        // Default civilian life points are set on `NpcData::default()`
        // (in `element.rs`) to the engine's `CIVILIAN_LIFE_POINTS = 100`.

        // `go_to_duty = init_state() && !ai_is_script_locked() && !ai_is_locked()`.
        // The `init_state` call commits the AI-side state
        // transition chosen by the level designer's authored
        // initial action and tells us whether the actor should
        // launch into its duty loop after.
        let fx = self.base.init_state(ctx);

        // `go_to_duty` is computed *before* the beggar-lock below,
        // so a beggar authored as `WaitingUpright` /
        // `WaitingUprightBored` / etc. still gets `go_to_duty=true`
        // and takes the else-branch's `launch_timer +
        // SetState(Default, OnPost)` cascade below.  (Re-reading
        // `ai_is_locked()` post-beggar-lock to gate the patrol-path
        // vs else branches is correct, and matches the downstream
        // check below.)
        let go_to_duty =
            fx.go_to_duty && !self.base.ai_is_script_locked() && !self.base.ai_is_locked();

        // Beggar civilians get a non-script `BEGGAR` lock so their
        // script-driven begging loop isn't interrupted by ambient
        // AI decisions.  This runs *after* `init_state` and *after*
        // `go_to_duty` is computed.
        if ctx.self_is_beggar {
            self.base.non_script_lock(crate::ai::AiLockFlags::BEGGAR);
        }

        if !self.base.ai_is_locked() && self.base.has_patrol_path {
            self.base.substate_at_last_timer_launch = self.base.current_substate;
            if go_to_duty {
                self.return_to_duty(DutyFlags::empty(), ctx);
            }
            // `GoTo` checks the think-method recursion depth and
            // either sets `already_on_point` (for the enclosing
            // `EndThink` to dispatch) or fires `Think(EventReachPoint)`
            // directly when called outside a Think cycle.
            // `return_to_duty` runs outside Think, so a `GoTo` to a
            // waypoint we already stand on sets `already_on_point =
            // true` but nothing drains it — queue a self-stimulus
            // so the engine's next-tick drain dispatches it (same
            // shape as the enemy branch).
            if self.base.already_on_point {
                self.base.already_on_point = false;
                self.base
                    .fire_self_stimulus(crate::ai::StimulusType::EventReachPoint);
            }
            if self.base.couldnt_reachpoint {
                self.base.couldnt_reachpoint = false;
                self.base
                    .fire_self_stimulus(crate::ai::StimulusType::EventCouldntReachPoint);
            }
            if self.base.already_turned {
                self.base.already_turned = false;
                self.base
                    .fire_self_stimulus(crate::ai::StimulusType::EventDone);
            }
        } else if go_to_duty {
            // Civilians without a patrol path and `go_to_duty=true`
            // get the authored "first look" randomised delay.
            // `init_state` already launched the bored timer via
            // its `WaitingUpright` branch, so we overwrite with the
            // longer look timer here — the second `launch_timer`
            // call wins.
            let timer_value =
                AB_MIN_DEFAULT_LOOK_TIME + crate::sim_rng::i32(0..AB_DELTA_DEFAULT_LOOK_TIME);
            self.base.launch_timer(timer_value as u32, ctx.frame);
            self.set_state(AiState::Default, Substate::DefaultOnPost);
            self.base.substate_at_last_timer_launch = self.base.current_substate;
        }

        fx
    }

    /// Propose a good destination for fleeing an apple chase.
    ///
    /// Algorithm:
    ///
    /// - Base direction = vector from antagonist to self, sectorised
    ///   to 0..15, with a `+rand(0..5) + 14` jitter (`-2..+2` mod 16).
    /// - For each target distance from `APPLE_CHASE_IDEAL_DISTANCE`
    ///   down to 20 in 10-unit steps, for each relative direction
    ///   in 0, +1, -1, +2, -2, … ±7: if the candidate straight-line
    ///   segment passes `FastFindGrid::is_straight_movement_authorized`,
    ///   return it.
    /// - Return `None` if every candidate is blocked.
    ///
    /// When `grid` is `None` (unit tests), the `is_straight_movement_authorized`
    /// check is skipped and the base-direction / ideal-distance
    /// candidate is accepted so callers still get a flee vector.
    pub fn propose_good_apple_chase_flee_destination(
        &self,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> Option<Position> {
        let antagonist = ctx.entity_view(self.base.antagonist)?;

        // Base direction is antagonist→self, jittered by
        // `(rand()%5) + 14` which is `-2..+2` mod 16.
        let dx = ctx.position.x - antagonist.position.x;
        let dy = ctx.position.y - antagonist.position.y;
        let base_dir = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy) as i32;
        let jitter = crate::sim_rng::u32(0..5) as i32;
        let seed_dir = (base_dir + jitter + 14).rem_euclid(16);

        // Relative direction sequence:
        // 0, 1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6, -6, 7, -7.
        let rel_sequence: [i32; 15] = [0, 1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6, -6, 7, -7];

        let origin_pt = crate::geo2d::pt(ctx.position.x, ctx.position.y);

        // Outer loop walks distance from
        // `APPLE_CHASE_IDEAL_DISTANCE` down to 20 in steps of -10.
        let mut distance = APPLE_CHASE_IDEAL_DISTANCE as f32;
        while distance > 10.0 {
            for &rel in &rel_sequence {
                // The legacy behavior uses `% 15` here (a source-
                // level bug — should be mod 16); reproduce it
                // faithfully so the flee vector matches the
                // original game.
                let dir = ((seed_dir + rel).rem_euclid(15)) as i16;
                // The iso sector-to-vector helper writes
                // `(tableX[idx], tableY[idx] * ASPECT_RATIO)` — the
                // Y-compressed unit vector that turns a screen-
                // sector index back into a map-space offset.  The
                // bare `direction_vector_16` would over-extend Y by
                // `1/AR` (≈1.74) and pick a different absolute
                // landing point.
                let [vx, vy] = crate::position_interface::sector_to_vector_iso(dir);
                let dest = Position {
                    x: ctx.position.x + vx * distance,
                    y: ctx.position.y + vy * distance,
                    sector: ctx.position.sector,
                    level: ctx.position.level,
                };

                // `is_straight_movement_authorized` rejects
                // candidates whose straight-line segment crosses a
                // motion obstacle.  Without a grid on the call
                // stack (unit tests), accept.
                let accepted = match grid {
                    Some(g) => g.is_straight_movement_authorized(
                        origin_pt,
                        crate::geo2d::pt(dest.x, dest.y),
                        ctx.position.level,
                        &ctx.move_box,
                    ),
                    None => true,
                };
                if accepted {
                    return Some(dest);
                }
            }
            distance -= 10.0;
        }

        // Every candidate blocked.
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_ai_defaults() {
        let ai = FriendlyAi::new(99);
        assert_eq!(ai.base.me, 99);
        assert_eq!(ai.base.current_state, AiState::Default);
        assert_eq!(ai.beggar_dont_talk_counter, 0);
    }

    #[test]
    fn civilian_set_state() {
        let mut ai = FriendlyAi::new(1);
        ai.set_state(AiState::Fleeing, Substate::FleeingPanic);
        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);
    }

    #[test]
    fn civilian_set_state_alert_levels() {
        let mut ai = FriendlyAi::new(1);

        ai.set_state(AiState::Default, Substate::DefaultOnPost);
        assert_eq!(ai.base.current_music_alert_status, AlertLevel::Green);

        ai.set_state(AiState::Wondering, Substate::WonderingCivilianAdmiringHero);
        assert_eq!(ai.base.current_music_alert_status, AlertLevel::Green);

        ai.set_state(AiState::Seeking, Substate::SeekingCivilianRunningToSoldier);
        assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);

        ai.set_state(AiState::Fleeing, Substate::FleeingPanic);
        assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);
    }

    #[test]
    fn civilian_return_to_duty() {
        let mut ai = FriendlyAi::new(1);
        ai.fleeing_seen_enemy_counter = 5;
        ai.set_state(AiState::Fleeing, Substate::FleeingPanic);
        ai.return_to_duty(DutyFlags::empty(), &AiContext::default());
        assert_eq!(ai.base.current_state, AiState::Default);
        // NPC walks back to initial position first, then transitions
        // to DefaultOnPost via EventReachPoint → DefaultGotoPostTurn → EventDone.
        assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
        assert_eq!(ai.fleeing_seen_enemy_counter, 0);
    }

    #[test]
    fn civilian_panic_from_point() {
        let mut ai = FriendlyAi::new(1);
        ai.panic_from_point(100.0, 200.0, 8);
        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        assert_eq!(ai.base.panic_center_x, 100.0);
        assert_eq!(ai.base.panic_center_y, 200.0);
        assert_eq!(ai.base.lasting_panic_runs, 8);
        assert!(ai.base.directed_panic);
        assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);
    }

    #[test]
    fn civilian_panic_undirected() {
        let mut ai = FriendlyAi::new(1);
        ai.panic_undirected(4, &AiContext::default());
        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        assert_eq!(ai.base.lasting_panic_runs, 4);
        assert!(!ai.base.directed_panic);
    }

    #[test]
    fn think_expected_admiring_hero_returns_to_duty() {
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(AiState::Wondering, Substate::WonderingCivilianAdmiringHero);

        let stimulus = Stimulus::new(StimulusType::EventTimer);
        ai.think_expected_event(
            &stimulus,
            &mut global,
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Default);
        // Walks back to post first (DefaultGotoPost → EventReachPoint → OnPost).
        assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
    }

    #[test]
    fn think_alerting_event_panic() {
        let mut ai = FriendlyAi::new(1);
        let pos = Position {
            x: 50.0,
            y: 75.0,
            sector: None,
            level: 0,
        };
        let stimulus = Stimulus::with_position(StimulusType::EventPanic, pos);

        ai.think_alerting_event(&stimulus, &AiContext::default(), None, None);

        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        assert_eq!(ai.base.panic_center_x, 50.0);
        assert_eq!(ai.base.panic_center_y, 75.0);
    }

    #[test]
    fn think_alerting_event_stop() {
        let mut ai = FriendlyAi::new(1);
        let stimulus = Stimulus::new(StimulusType::EventStop);

        ai.think_alerting_event(&stimulus, &AiContext::default(), None, None);

        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(ai.base.current_substate, Substate::SeekingGotStopEvent);
        assert!(ai.base.timer_is_running);
    }

    #[test]
    fn think_alerting_event_stop_while_sleeping() {
        let mut ai = FriendlyAi::new(1);
        ai.set_state(AiState::Sleeping, Substate::SleepingForever);
        let stimulus = Stimulus::new(StimulusType::EventStop);

        let result = ai.think_alerting_event(&stimulus, &AiContext::default(), None, None);

        // Should return false and NOT change state
        assert!(!result);
        assert_eq!(ai.base.current_state, AiState::Sleeping);
    }

    #[test]
    fn think_unexpected_couldnt_reachpoint_returns_to_duty() {
        let mut ai = FriendlyAi::new(1);
        ai.set_state(AiState::Seeking, Substate::SeekingCivilianRunningToSoldier);

        let stimulus = Stimulus::new(StimulusType::EventCouldntReachPoint);
        ai.think_unexpected_event(&stimulus, &AiContext::default(), None, None);

        assert_eq!(ai.base.current_state, AiState::Default);
        // Walks back to post first.
        assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
    }

    #[test]
    fn think_unexpected_fit_again_returns_to_duty() {
        let mut ai = FriendlyAi::new(1);
        ai.set_state(AiState::Sleeping, Substate::SleepingUnconscious);

        let stimulus = Stimulus::new(StimulusType::EventFitAgain);
        ai.think_unexpected_event(&stimulus, &AiContext::default(), None, None);

        assert_eq!(ai.base.current_state, AiState::Default);
    }

    #[test]
    fn fit_again_queues_resurrection_and_eye_reset() {
        // EVENT_FITAGAIN must fire the resurrection fan-out and
        // reset the view status to LookForward alongside the
        // return-to-duty hand-off.  Both are surfaced via pending
        // flags on `AiController`; this test pins them so the
        // engine drain keeps firing them.
        let mut ai = FriendlyAi::new(1);
        ai.set_state(AiState::Sleeping, Substate::SleepingUnconscious);

        let stimulus = Stimulus::new(StimulusType::EventFitAgain);
        ai.think_unexpected_event(&stimulus, &AiContext::default(), None, None);

        assert!(
            ai.base.pending_inform_resurrection,
            "EVENT_FITAGAIN must queue resurrection fan-out"
        );
        assert_eq!(
            ai.base.pending_set_eye_status,
            Some(crate::element::EyeStatus::LookForward),
            "EVENT_FITAGAIN must reset eyes to LookForward"
        );
    }

    #[test]
    fn fleeing_event_view_uses_directed_panic_from_human_position() {
        // EVENT_VIEW while fleeing must fire a *directed* panic
        // away from the spotted human.  An earlier port used
        // `panic_undirected` which lost the center and the civilian
        // picked a random door instead of fleeing opposite the
        // threat.
        use crate::ai_entity_view::{AiEntityView, AiEntityViewMap, EntityKind};
        use crate::element::{Camp, Posture};
        use crate::order::OrderType;
        let mut ai = FriendlyAi::new(1);
        ai.set_state(AiState::Fleeing, Substate::FleeingRunToDoor);

        let human_handle: u32 = 42;
        let enemy_pos = Position {
            x: 150.0,
            y: 250.0,
            sector: None,
            level: 0,
        };
        let mut views = AiEntityViewMap::new();
        views.insert(
            human_handle,
            AiEntityView {
                position: enemy_pos,
                direction: 0,
                posture: Posture::Upright,
                camp: Camp::Lacklandists, // different from default (Royalists)
                is_pc: false,
                is_robin: false,
                is_vip: false,
                is_beggar: false,
                is_child: false,
                kind: EntityKind::Soldier,
                is_tower_guard: false,
                is_swordfighting: false,
                is_able_to_fight: true,
                is_unconscious: false,
                in_building: false,
                building_sector: None,
                ai_state: AiState::Default,
                ai_substate: Substate::DefaultOnPost,
                script_locked: false,
                forecasted_destination: enemy_pos,
                current_animation: OrderType::WalkingUpright,
                elevation: 0.0,
                object_type: crate::element_kinds::ObjectType::None,
                is_dead: false,
                is_carried: false,
                is_archer: false,
                is_rider: false,
                stuck_under_net: false,
                in_coma: false,
                guard: None,
                has_patrol_path: false,
                initial_position: enemy_pos,
                number_of_arrows: 0,
                covering_nets: Vec::new(),
                rank: crate::profiles::ProfileRank::None,
                reported_to_officer: false,
                looted_after_money_fight: false,
                current_money: 0,
                macro_in_progress: false,
                path_current_waypoint_index: 0,
                path_last_waypoint_index: 0,
                path_forward_movement: true,
                patrol_hiking_path_index: None,
                interesting_object: 0,
                report_type: crate::ai::ReportType::Nothing,
                report_seek_position: enemy_pos,
                report_seen_bodies: Vec::new(),
                report_charly: 0,
            },
        );
        let ctx = AiContext {
            camp: Camp::Royalists,
            entity_views: std::sync::Arc::new(views),
            ..AiContext::default()
        };

        let stimulus = Stimulus::with_human(StimulusType::EventView, human_handle);
        ai.think_alerting_event(&stimulus, &ctx, None, None);

        assert!(
            ai.base.directed_panic,
            "EVENT_VIEW while fleeing must fire a *directed* panic"
        );
        let request = ai
            .base
            .pending_begin_panic
            .as_ref()
            .expect("a panic request must be queued");
        let center = request
            .center
            .expect("directed panic must carry a center point");
        assert_eq!(center.x, enemy_pos.x);
        assert_eq!(center.y, enemy_pos.y);
    }

    #[test]
    fn think_unexpected_net_away_panics() {
        let mut ai = FriendlyAi::new(1);
        let stimulus = Stimulus::new(StimulusType::EventNetAway);

        ai.think_unexpected_event(&stimulus, &AiContext::default(), None, None);

        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    }

    #[test]
    fn event_sees_body_sets_wondering_state() {
        let mut ai = FriendlyAi::new(1);
        ai.event_sees_body_standard_procedure(42, &AiContext::default());
        assert_eq!(ai.base.current_state, AiState::Wondering);
        assert_eq!(
            ai.base.current_substate,
            Substate::WonderingCivilianBodyReactiontime,
        );
        assert_eq!(ai.base.current_remark, Remark::CivSeesBody);
        assert_eq!(
            ai.base.my_reconnaissance_report.report_type,
            ReportType::Body
        );
    }

    #[test]
    fn event_sees_object_is_noop() {
        let mut ai = FriendlyAi::new(1);
        ai.event_sees_object_standard_procedure(42);
        assert_eq!(ai.base.current_state, AiState::Default);
    }

    #[test]
    fn update_new_task_priority_is_noop() {
        let mut ai = FriendlyAi::new(1);
        let stimulus = Stimulus::new(StimulusType::EventTimer);
        ai.update_new_task_priority(&stimulus);
        // Should not panic or change state
    }

    #[test]
    fn expected_event_body_reactiontime_alert_fails_panics() {
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(
            AiState::Wondering,
            Substate::WonderingCivilianBodyReactiontime,
        );

        let stimulus = Stimulus::new(StimulusType::EventTimer);
        ai.think_expected_event(
            &stimulus,
            &mut global,
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
            None,
        );

        // AlertSoldier returns false (stub) → should panic
        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        assert_eq!(ai.base.current_remark, Remark::CivPanic);
    }

    #[test]
    fn expected_event_whistling_child_approaches() {
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(AiState::Wondering, Substate::WonderingWatchingWhistling);

        let stimulus = Stimulus::new(StimulusType::EventTimer);
        ai.think_expected_event(
            &stimulus,
            &mut global,
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Wondering);
        assert_eq!(
            ai.base.current_substate,
            Substate::WonderingChildApproachingWhistling,
        );
        assert_eq!(ai.base.current_remark, Remark::CivWhistling);
    }

    #[test]
    fn fleeing_child_chased_end_returns_to_duty() {
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(AiState::Fleeing, Substate::FleeingChildChasedEnd);

        let stimulus = Stimulus::new(StimulusType::EventTimer);
        ai.think_expected_event(
            &stimulus,
            &mut global,
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Default);
    }

    #[test]
    fn seeking_report_point_done_transitions() {
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(
            AiState::Seeking,
            Substate::SeekingCivilianGiveAlertingReportToSoldierPoint,
        );

        let stimulus = Stimulus::new(StimulusType::EventDone);
        ai.think_expected_event(
            &stimulus,
            &mut global,
            &AiContext::default(),
            &AiPerTickData::stub(),
            None,
            None,
        );

        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingCivilianGiveAlertingReportToSoldierEnd,
        );
        assert!(ai.base.timer_is_running);
    }

    // ──────────────────────────────────────────────────────────
    // AlertSoldier body-level divergence fixes
    // ──────────────────────────────────────────────────────────

    fn make_soldier_view(
        pos: Position,
        camp: crate::element::Camp,
        ai_state: AiState,
    ) -> crate::ai_entity_view::AiEntityView {
        use crate::ai_entity_view::EntityKind;
        use crate::element::Posture;
        use crate::order::OrderType;
        crate::ai_entity_view::AiEntityView {
            position: pos,
            direction: 0,
            posture: Posture::Upright,
            camp,
            is_pc: false,
            is_robin: false,
            is_vip: false,
            is_beggar: false,
            is_child: false,
            kind: EntityKind::Soldier,
            is_tower_guard: false,
            is_swordfighting: false,
            is_able_to_fight: true,
            is_unconscious: false,
            in_building: false,
            building_sector: None,
            ai_state,
            ai_substate: Substate::DefaultOnPost,
            script_locked: false,
            forecasted_destination: pos,
            current_animation: OrderType::WalkingUpright,
            elevation: 0.0,
            object_type: crate::element_kinds::ObjectType::None,
            is_dead: false,
            is_carried: false,
            is_archer: false,
            is_rider: false,
            stuck_under_net: false,
            in_coma: false,
            guard: None,
            has_patrol_path: false,
            initial_position: pos,
            number_of_arrows: 0,
            covering_nets: Vec::new(),
            rank: crate::profiles::ProfileRank::None,
            reported_to_officer: false,
            looted_after_money_fight: false,
            current_money: 0,
            macro_in_progress: false,
            path_current_waypoint_index: 0,
            path_last_waypoint_index: 0,
            path_forward_movement: true,
            patrol_hiking_path_index: None,
            interesting_object: 0,
            report_type: crate::ai::ReportType::Nothing,
            report_seek_position: pos,
            report_seen_bodies: Vec::new(),
            report_charly: 0,
        }
    }

    #[test]
    fn alert_soldier_short_circuits_on_nearby_alerted_friend() {
        // Any same-camp soldier in ATTACKING/MENACING/FLEEING
        // within the 360° view radius short-circuits the alert —
        // no point running to a second soldier when one next door
        // is already alerted.
        use crate::ai_entity_view::AiEntityViewMap;
        use crate::element::Camp;
        let mut ai = FriendlyAi::new(1);

        let alerted_pos = Position {
            x: 10.0,
            y: 10.0,
            sector: None,
            level: 0,
        };
        let default_pos = Position {
            x: 500.0,
            y: 500.0,
            sector: None,
            level: 0,
        };

        let mut views = AiEntityViewMap::new();
        views.insert(
            10,
            make_soldier_view(alerted_pos, Camp::Royalists, AiState::Attacking),
        );
        views.insert(
            20,
            make_soldier_view(default_pos, Camp::Royalists, AiState::Default),
        );
        let ctx = AiContext {
            position: Position {
                x: 0.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            camp: Camp::Royalists,
            // Large enough so the alerted soldier is "detected 360°"
            sq_standard_view_radius: 1_000_000.0,
            entity_views: std::sync::Arc::new(views),
            ..AiContext::default()
        };

        let ok = ai.alert_soldier(ctx.position, 0, &ctx, None, None);
        assert!(
            !ok,
            "alert_soldier must return false when alerted friend nearby"
        );
        // State must not have switched to seeking.
        assert_eq!(ai.base.current_state, AiState::Default);
    }

    #[test]
    fn alert_soldier_applies_layer_penalty() {
        // +1000 MaxNorm penalty for soldiers on a different layer.
        // A closer same-layer candidate should win over a nominally-
        // nearer cross-layer one.
        use crate::ai_entity_view::AiEntityViewMap;
        use crate::element::Camp;
        let mut ai = FriendlyAi::new(1);

        let close_cross_layer = Position {
            x: 100.0,
            y: 0.0,
            sector: None,
            level: 1, // different layer → +1000 penalty
        };
        let farther_same_layer = Position {
            x: 300.0,
            y: 0.0,
            sector: None,
            level: 0,
        };

        let mut views = AiEntityViewMap::new();
        views.insert(
            10,
            make_soldier_view(close_cross_layer, Camp::Royalists, AiState::Default),
        );
        views.insert(
            20,
            make_soldier_view(farther_same_layer, Camp::Royalists, AiState::Default),
        );
        let ctx = AiContext {
            position: Position {
                x: 0.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            camp: Camp::Royalists,
            sq_standard_view_radius: 1.0, // too small for short-circuit
            entity_views: std::sync::Arc::new(views),
            ..AiContext::default()
        };

        crate::sim_rng::with_seed(1, || {
            let ok = ai.alert_soldier(ctx.position, 0, &ctx, None, None);
            assert!(ok, "alert_soldier must succeed when at least one candidate");
            // Antagonist must be the same-layer one despite being farther.
            assert_eq!(ai.base.antagonist, 20);
        });
    }

    #[test]
    fn alert_soldier_queues_friend_detectables_on_first_pass() {
        // Every candidate soldier gets a DETECTABLE_FRIEND add on
        // the non-door-path pass so later "is my ally still
        // nearby?" checks light up.
        use crate::ai_entity_view::AiEntityViewMap;
        use crate::element::{Camp, DetectableType};
        let mut ai = FriendlyAi::new(1);

        let mut views = AiEntityViewMap::new();
        views.insert(
            10,
            make_soldier_view(
                Position {
                    x: 100.0,
                    y: 0.0,
                    sector: None,
                    level: 0,
                },
                Camp::Royalists,
                AiState::Default,
            ),
        );
        let ctx = AiContext {
            position: Position {
                x: 0.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            camp: Camp::Royalists,
            sq_standard_view_radius: 1.0,
            entity_views: std::sync::Arc::new(views),
            ..AiContext::default()
        };

        crate::sim_rng::with_seed(1, || {
            ai.alert_soldier(ctx.position, 0, &ctx, None, None);
            let friends: Vec<_> = ai
                .base
                .pending_add_detectables
                .iter()
                .filter(|(_, t)| *t == DetectableType::Friend)
                .collect();
            assert!(
                !friends.is_empty(),
                "alert_soldier must queue DETECTABLE_FRIEND adds on first pass"
            );
        });
    }

    // ──────────────────────────────────────────────────────────
    // Apple-chase flee: full scan, not a single-guess stub
    // ──────────────────────────────────────────────────────────

    #[test]
    fn propose_apple_chase_flee_returns_candidate_without_grid() {
        // Without a grid the `is_straight_movement_authorized`
        // check is skipped and the first-distance / zero-relative
        // candidate wins.  Verifies the happy path still produces a
        // flee destination.
        use crate::ai_entity_view::AiEntityViewMap;
        use crate::element::Camp;
        let mut ai = FriendlyAi::new(1);
        ai.base.antagonist = 42;

        let mut views = AiEntityViewMap::new();
        views.insert(
            42,
            make_soldier_view(
                Position {
                    x: 100.0,
                    y: 0.0,
                    sector: None,
                    level: 0,
                },
                Camp::Royalists,
                AiState::Wondering,
            ),
        );
        let ctx = AiContext {
            position: Position {
                x: 0.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            camp: Camp::Royalists,
            entity_views: std::sync::Arc::new(views),
            ..AiContext::default()
        };

        crate::sim_rng::with_seed(1, || {
            let dest = ai.propose_good_apple_chase_flee_destination(&ctx, None);
            assert!(dest.is_some());
            // Flee vector should point away from antagonist at x=100
            // → destination x should be negative.
            let d = dest.unwrap();
            assert!(
                d.x < ctx.position.x,
                "flee vector must run away from antagonist"
            );
        });
    }
}
