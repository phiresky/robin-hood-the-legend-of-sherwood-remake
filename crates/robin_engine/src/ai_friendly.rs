//! Friendly (civilian) AI.
//!
//! This module contains the `FriendlyAi` struct which extends [`AiController`]
//! with civilian-specific state: talking, panic behavior, beggar interactions,
//! and the civilian Think state machine.

use serde::{Deserialize, Serialize};

use crate::ai::*;
use crate::coordinates::MapPoint;
use crate::parameters_ai::{
    AB_DELTA_DEFAULT_LOOK_TIME, AB_MIN_DEFAULT_LOOK_TIME, AI_FIRST_LOOK_TIME,
    AI_STANDARD_PANIC_RUNS, AI_TALK_DISTANCE,
};

// ---------------------------------------------------------------------------
// Civilian-specific constants
// ---------------------------------------------------------------------------

pub const APPLE_CHASE_IDEAL_DISTANCE: i32 = 300;
pub const BEGGAR_NO_RANDOM_TALK_DISTANCE: i32 = 100;

/// Truthful engine snapshot for the only cross-entity per-tick value consumed
/// by Friendly AI. Deliberately has no `Default`/`stub`: handlers that require
/// a patrol chief must demand the live snapshot contextually.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct FriendlyPerTickData {
    patrol_chief: Option<FriendlyPatrolChief>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct FriendlyPatrolChief {
    position: Position,
    state: AiState,
}

impl FriendlyPerTickData {
    pub(crate) fn without_patrol_chief() -> Self {
        Self { patrol_chief: None }
    }

    pub(crate) fn with_patrol_chief(position: Position, state: AiState) -> Self {
        Self {
            patrol_chief: Some(FriendlyPatrolChief { position, state }),
        }
    }

    fn required_patrol_chief(self, owner: NpcHandle) -> FriendlyPatrolChief {
        self.patrol_chief.unwrap_or_else(|| {
            panic!("Friendly AI owner {owner} requires a live patrol-chief snapshot")
        })
    }
}

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
    /// Whether this civilian still accepts the talk interaction.
    pub wants_to_talk: bool,
    /// Last NPC this civilian talked to; zero is Original's null pointer.
    pub last_talk_partner: crate::ai::NpcHandle,
    /// Script-controlled permission to leave after the current interaction.
    pub can_go_away: bool,
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
            wants_to_talk: true,
            last_talk_partner: 0,
            can_go_away: true,
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
        debug_assert_eq!(
            substate.ai_state_family(),
            Some(state),
            "FriendlyAi::set_state received mismatched state/substate: {state:?}/{substate:?}"
        );

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
        // Calls made before virtual SetState belong inside its synchronous
        // callback boundary. In particular, common CoordinatePatrol falls
        // through from StopAll, so its Halt must be applied before the
        // civilian FilterAIEvent while the following GoTo remains outside.
        let actor_effects_before_callback = self
            .base
            .outbox
            .actor
            .has_boundary_work()
            .then(|| std::mem::take(&mut self.base.outbox.actor));
        self.base
            .outbox
            .reentrant
            .owner_work
            .push(AiOwnerWork::StateChange(AiStateChangeNotification {
                outgoing_state: self.base.current_state,
                outgoing_substate: self.base.current_substate,
                incoming_state: state,
                incoming_substate: substate,
                source,
                actor_effects_before_callback,
            }));

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

    /// Apply common patrol geometry through Bonhomie's virtual `SetState`.
    /// The base routine owns StopAll and formation planning; the friendly
    /// override owns alert/script state effects before the movement order.
    fn coordinate_patrol(
        &mut self,
        info: &StimulusInfo,
        ctx: &AiContext,
        patrol_chief_position: Position,
    ) {
        let Some(action) = self
            .base
            .prepare_patrol_coordinate(info, ctx, patrol_chief_position)
        else {
            return;
        };

        match action {
            PatrolCoordinateAction::FaceChief { target } => {
                self.base.face_position_with_ctx(target, ctx);
            }
            PatrolCoordinateAction::Walk {
                target,
                speed_factor,
            } => {
                let flags = GotoFlags::NO_HALT
                    | GotoFlags::DONT_STOP
                    | self.base.default_path_walking_flags;
                self.go_to_speed(
                    AiState::Default,
                    Substate::DefaultPatrolEnroute,
                    target,
                    flags,
                    speed_factor,
                    ctx,
                );
            }
            PatrolCoordinateAction::Run { target } => {
                self.go_to(
                    AiState::Default,
                    Substate::DefaultPatrolEnrouteRunning,
                    target,
                    GotoFlags::RUN | GotoFlags::NO_HALT | GotoFlags::DONT_STOP,
                    ctx,
                );
            }
        }
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
        // Original stages SetState only after its door search and only when
        // `bNewPanic` remains true. Preserve the eager fallback staging used
        // by new Rust requests, but do not let a repeated panic re-enter the
        // civilian state setter and lower an existing red alert to yellow.
        if !was_already_fleeing {
            self.set_state(AiState::Fleeing, Substate::FleeingPanic);
        }
        self.base.outbox.actor.begin_panic = Some(PanicRequest {
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
        if !was_already_fleeing {
            self.set_state(AiState::Fleeing, Substate::FleeingPanic);
        }
        self.base.outbox.actor.begin_panic = Some(PanicRequest {
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
    pub(crate) fn think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &FriendlyPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        doors: Option<&[crate::gate::Door]>,
    ) -> bool {
        self.base.cached_frame = ctx.frame;
        self.base.cached_in_building = ctx.in_building;

        let stimulus_type = stimulus.stimulus_type;

        self.base
            .register_log_line(LogLineType::Event, stimulus_type as u16);

        // Pre-think checks
        if !self.start_think(stimulus, ctx, global.freeze) {
            if stimulus_type == StimulusType::EventAfterScriptGoOn {
                self.base.outbox.reentrant.engine_drains_after_script_go_on = false;
            }
            self.end_think(sim, global, ctx);
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
                self.think_expected_event(sim, stimulus, global, ctx, tick, grid, doors)
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
            | StimulusType::EventNetAway => {
                self.think_unexpected_event(sim, stimulus, global, ctx, tick, grid, doors)
            }

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
            | StimulusType::EventStop => self.think_alerting_event(sim, stimulus, ctx, grid, doors),

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
                self.return_to_duty(sim, DutyFlags::empty(), ctx);
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

        if !(stimulus_type == StimulusType::EventAfterScriptGoOn
            && self.base.outbox.reentrant.engine_drains_after_script_go_on)
        {
            self.end_think(sim, global, ctx);
        }
        return_value
    }

    pub(crate) fn resolve_alert_request(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        accepted: bool,
        continuation: AlertContinuation,
        ctx: &AiContext,
    ) {
        if !accepted {
            self.panic_undirected(AI_STANDARD_PANIC_RUNS as u8, ctx);
            return;
        }

        if matches!(continuation, AlertContinuation::CivilianReachedSoldier) {
            self.base
                .outbox
                .actor
                .delete_detectables
                .push(crate::element::DetectableType::Friend);
        }
        self.set_state(
            AiState::Seeking,
            Substate::SeekingCivilianRunningToSoldierSeen,
        );

        match continuation {
            AlertContinuation::CivilianReachedSoldier => self
                .base
                .outbox
                .reentrant
                .self_stimuli
                .push(StimulusType::EventReachPoint),
            AlertContinuation::CivilianSawSoldier => {
                self.base.say(Remark::CivCallsSoldier);
                let target = self.base.antagonist;
                let target_pos = ctx
                    .entity_view(target)
                    .unwrap_or_else(|| {
                        panic!(
                            "accepted civilian alert from {} requires target soldier {} view",
                            self.base.me, target
                        )
                    })
                    .forecasted_destination
                    .resolve(sim)
                    .position;
                self.base
                    .go_near(target_pos, AI_TALK_DISTANCE, GotoFlags::RUN, ctx);
                self.base.launch_timer(20, ctx.frame);
            }
            AlertContinuation::SoldierSawOfficer => {
                panic!("civilian alert resolver received soldier continuation")
            }
        }
    }

    // -----------------------------------------------------------------------
    // Think sub-methods
    // -----------------------------------------------------------------------

    fn start_think(
        &mut self,
        stimulus: &Stimulus,
        ctx: &AiContext,
        static_ai_frozen: bool,
    ) -> bool {
        self.start_think_pre_filter(stimulus);
        self.start_think_post_filter(stimulus, ctx, static_ai_frozen)
    }

    /// `StartThink` work which precedes the script `FilterAIEvent` call.
    pub(crate) fn start_think_pre_filter(&mut self, stimulus: &Stimulus) {
        // Civilian pre-think pipeline.  Civilians normally never
        // hit `EventWasp` / `EventNet`, but the gates live on the
        // base class so any scripted `SetSubstate` could reach them;
        // mirror the enemy path's defensive refusals.
        let stimulus_type = stimulus.stimulus_type;

        self.base.couldnt_reachpoint = false;
        self.base.already_on_point = false;
        self.base.already_turned = false;
        self.base.old_state = self.base.current_state as i32;
        self.base.think_recursion_depth = self.base.think_recursion_depth.saturating_add(1);

        if let StimulusInfo::Human(h) = stimulus.info {
            self.base.last_stimulus_actor = Some(h);
        }

        // LOSE_CONSCIOUSNESS always drops the alert regardless of the
        // downstream refusal — even when the event is otherwise
        // filtered out.
        if stimulus_type == StimulusType::EventLoseConsciousness {
            self.base.set_alert_status(AlertLevel::Green);
        }
    }

    /// `StartThink` work after `FilterAIEvent`. SetAIState observes these
    /// gates but deliberately ignores the returned admission decision.
    pub(crate) fn start_think_post_filter(
        &mut self,
        stimulus: &Stimulus,
        ctx: &AiContext,
        static_ai_frozen: bool,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;

        self.base.couldnt_reachpoint = false;
        self.base.already_on_point = false;
        self.base.already_turned = false;

        // Static AI freeze discards stimuli after the engine-side script
        // filter. It is not the per-NPC AILOCK_FREEZE retention bit.
        if static_ai_frozen {
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

        // Every non-script AILOCK flag retains stimuli. Original's separate
        // static `mbFreeze` discard gate is not the per-NPC AILOCK_FREEZE bit.
        if !self.base.locks_flag_field.is_empty() {
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

        // These three stimuli are consumed by the common
        // RHArtificialIntelligence::StartThink implementation before the
        // civilian-specific Think dispatcher runs.  They therefore mutate
        // the base AI even though FriendlyAi's alerting-event switch has no
        // derived handling for them.
        match stimulus_type {
            StimulusType::EventLoseConsciousness => {
                self.base.break_macro();
                self.base.clear_emoticon();
                self.set_state(AiState::Sleeping, Substate::SleepingUnconscious);
                self.base.outbox.recovery.set_eye_status =
                    Some(crate::element::EyeStatus::DieOrGetUnconscious);
                self.base.set_alert_status(AlertLevel::Green);
                self.base.sorrow_level = 0;
                self.base.register_log_line(LogLineType::EventRefused, 13);
                return false;
            }
            StimulusType::EventWasp => {
                self.base.break_macro();
                self.base.set_emoticon(EmoticonType::Thunderstorm);
                self.set_state(AiState::Wondering, Substate::WonderingWaspInArmour);
                self.base.outbox.recovery.set_eye_status = Some(crate::element::EyeStatus::Closed);
                self.base.sorrow_level = 0;
                self.base.register_log_line(LogLineType::EventRefused, 14);
                return false;
            }
            StimulusType::EventNet => {
                self.base.break_macro();
                self.set_state(AiState::Wondering, Substate::WonderingUnderNet);
                self.base.outbox.recovery.set_eye_status = Some(crate::element::EyeStatus::Closed);
                self.base.sorrow_level = 0;
                self.base.register_log_line(LogLineType::EventRefused, 15);
                return false;
            }
            _ => {}
        }

        true
    }

    pub(crate) fn end_think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
    ) {
        // legacy implementation EndThink calls Think(EVENT_*) here, and Think runs the
        // script FilterAIEvent gate before dispatch. Queue these as
        // same-frame self-stimuli so the engine-side drain can apply
        // that filter without re-entering the script VM through this
        // borrowed AI object. The three-tier depth gate still matches
        // legacy implementation: <100 queues the follow-up, 100..=110 bails to
        // `ReturnToDuty`, 111+ drops it silently.

        if self.base.think_recursion_depth < 100 {
            // Dispatching a completion re-enters Think, and Think's entry gate
            // clears all three latches before the nested handler runs. Only
            // the first set latch can therefore survive to be dispatched.
            let event = if self.base.couldnt_reachpoint {
                Some(StimulusType::EventCouldntReachPoint)
            } else if self.base.already_on_point {
                Some(StimulusType::EventReachPoint)
            } else if self.base.already_turned {
                Some(StimulusType::EventDone)
            } else {
                None
            };
            self.base.couldnt_reachpoint = false;
            self.base.already_on_point = false;
            self.base.already_turned = false;
            if let Some(event) = event {
                self.base.outbox.reentrant.self_stimuli.push(event);
            }
        } else {
            // The deep-recursion fallback runs `ReturnToDuty` instead of a
            // nested Think, so it never clears the sibling latches and each
            // one falls back independently.
            let couldnt_reachpoint = std::mem::take(&mut self.base.couldnt_reachpoint);
            let already_on_point = std::mem::take(&mut self.base.already_on_point);
            let already_turned = std::mem::take(&mut self.base.already_turned);
            if self.base.think_recursion_depth < 111 {
                for pending in [couldnt_reachpoint, already_on_point, already_turned] {
                    if pending {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx);
                    }
                }
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
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &FriendlyPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        doors: Option<&[crate::gate::Door]>,
    ) -> bool {
        debug_assert_eq!(
            self.base.current_substate.ai_state_family(),
            Some(self.base.current_state),
            "FriendlyAi expected-event dispatch received mismatched state/substate: {:?}/{:?}",
            self.base.current_state,
            self.base.current_substate
        );

        let stimulus_type = stimulus.stimulus_type;

        match self.base.current_substate {
            // -------- Common stuff for soldiers and civilians --------
            Substate::FleeingPanic => {
                if self.base.lasting_panic_runs == 0 {
                    self.fleeing_seen_enemy_counter = 0;
                }
                // Falls through to the common-stuff dispatcher.
                return self
                    .base
                    .think_expected_event_common_stuff(sim, stimulus, ctx);
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
                return self
                    .base
                    .think_expected_event_common_stuff(sim, stimulus, ctx);
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
                    if self.base.patrol_direction != ctx.direction {
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
                    match tick.required_patrol_chief(self.base.me).state {
                        AiState::Default | AiState::Wondering => {
                            self.base.launch_timer(200, ctx.frame);
                        }
                        _ => {
                            self.return_to_duty(sim, DutyFlags::empty(), ctx);
                        }
                    }
                }
            }

            Substate::DefaultChildApproachedWhistling => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx);
                }
            }

            // ############## W O N D E R I N G #####################
            Substate::WonderingCivilianAdmiringHero => {
                if stimulus_type == StimulusType::EventTimer {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx);
                }
            }

            Substate::WonderingCivilianEnemyReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    let seek_pos = self.base.seek_position;
                    if !self.alert_soldier(sim, seek_pos, 0, ctx, grid, doors) {
                        self.base.say(Remark::CivPanic);
                        let pos = self.base.seek_position;
                        self.panic_from_position(pos, AI_STANDARD_PANIC_RUNS as u8, ctx);
                    }
                }
            }

            Substate::WonderingCivilianBodyReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    let seek_pos = self.base.seek_position;
                    if !self.alert_soldier(sim, seek_pos, 0, ctx, grid, doors) {
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
                    let antagonist_view = ctx.entity_view(antagonist_handle).unwrap_or_else(|| {
                        panic!(
                            "civilian {} running to required antagonist {} has no entity view",
                            self.base.me, antagonist_handle
                        )
                    });
                    match antagonist_view.ai_state {
                        AiState::Default => {
                            // "You have not seen the officer!" — the
                            // soldier is still on duty, so close the
                            // last few steps and talk to them.  The
                            // outer match arm already proves the
                            // view is `Some(…)` here, so the unwrap
                            // is infallible.
                            let antag_view = antagonist_view;
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
                                    antag_view.forecasted_destination.resolve(sim).position,
                                    AI_TALK_DISTANCE,
                                    GotoFlags::RUN,
                                    ctx,
                                );
                            } else {
                                self.base.outbox.reentrant.cross_npc_actions.push(
                                    CrossNpcAction::RequestAlert {
                                        target: antagonist_handle,
                                        caller: self.base.me,
                                        continuation:
                                            crate::ai::AlertContinuation::CivilianReachedSoldier,
                                    },
                                );
                            }
                        }
                        _ => {
                            // Officer is no longer in STATE_DEFAULT
                            // (reassigned / knocked out / script
                            // interrupted) — look for another soldier and,
                            // on failure, fall back to `ReturnToDuty`.
                            let seek_pos = self.base.seek_position;
                            if !self.alert_soldier(sim, seek_pos, 0, ctx, grid, doors) {
                                self.return_to_duty(sim, DutyFlags::empty(), ctx);
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
                            self.return_to_duty(sim, DutyFlags::empty(), ctx);
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
                            self.return_to_duty(sim, DutyFlags::empty(), ctx);
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
                    // synchronous inter-NPC Think boundary. We pass a
                    // Hint carrying our seek point so the soldier's
                    // CALL_REPORT handler can update its own report
                    // without needing to reach back into the
                    // civilian's AI state.  The return value is
                    // ignored — it's fire-and-forget.
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::SendStimulus {
                            target: self.base.antagonist,
                            stimulus_type: StimulusType::CallReport,
                            info: StimulusInfo::Hint(Hint {
                                seek_point: self.base.seek_position,
                                seek_flags: 0,
                                who_tells_me: self.base.me,
                            }),
                            fallback_to_sender: None,
                            to_whole_patrol: false,
                        },
                    );
                    self.base.say(Remark::CivDenunciates);
                    let seek_pos = self.base.seek_position;
                    self.base.point_to(seek_pos, ctx);
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
                    self.return_to_duty(sim, DutyFlags::empty(), ctx);
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
                            self.propose_good_apple_chase_flee_destination(sim, ctx, grid)
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
                            self.propose_good_apple_chase_flee_destination(sim, ctx, grid)
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
                    self.return_to_duty(sim, DutyFlags::empty(), ctx);
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

    pub(crate) fn think_unexpected_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &FriendlyPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        doors: Option<&[crate::gate::Door]>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;

        match stimulus_type {
            StimulusType::EventSeesSoldier
                if self.base.current_substate == Substate::SeekingCivilianRunningToSoldier =>
            {
                if let StimulusInfo::Human(soldier_handle) = stimulus.info {
                    self.base.antagonist = soldier_handle;
                    // The original deletes friend detectables before the
                    // direct CALL_ALERT, including when the soldier refuses.
                    self.base
                        .outbox
                        .actor
                        .delete_detectables
                        .push(crate::element::DetectableType::Friend);
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::RequestAlert {
                            target: soldier_handle,
                            caller: self.base.me,
                            continuation: crate::ai::AlertContinuation::CivilianSawSoldier,
                        },
                    );
                }
            }

            StimulusType::CallPatrolCoordinate => {
                self.coordinate_patrol(
                    &stimulus.info,
                    ctx,
                    tick.required_patrol_chief(self.base.me).position,
                );
            }

            StimulusType::EventAfterScriptGoOn => {
                if self.base.outbox.reentrant.engine_drains_after_script_go_on {
                    return false;
                }
                // Drain retained stimuli exactly as the Original's recursive
                // Think(stimulus) loop does. Preserve the complete stimulus:
                // reducing an EventView to its type discards the viewed actor
                // and turns the remembered event into a silent no-op.
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
                        // `Think(stimulus)` receives the queued stimulus's live
                        // human pointer in the Original. Rust carries that
                        // target-specific view separately on `AiContext`, so
                        // the outer EVENT_AFTER_SCRIPT_GO_ON context cannot be
                        // reused unchanged for a retained EVENT_VIEW.
                        let mut nested_ctx = ctx.clone();
                        if let StimulusInfo::Human(handle) = q.info {
                            let view = nested_ctx.entity_view(handle).unwrap_or_else(|| {
                                panic!(
                                    "retained {:?} for civilian {} references missing human {}",
                                    q.stimulus_type, self.base.me, handle
                                )
                            });
                            nested_ctx.antagonist = Some(crate::ai::AntagonistInfo {
                                position: view.position,
                                camp: view.camp,
                                is_swordfighting: view.is_swordfighting,
                                is_pc: view.is_pc,
                                is_robin: view.is_robin,
                                is_vip: view.is_vip,
                                in_building: view.in_building,
                            });
                        }
                        self.think(sim, &q, global, &nested_ctx, tick, grid, doors);
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
                            self.return_to_duty(sim, DutyFlags::empty(), ctx);
                        }
                    } else {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx);
                    }
                    return false;
                }
            }

            StimulusType::CallYouJustWait => {
                // Soldier tells child to wait (apple chase begins)
                if let StimulusInfo::Human(soldier_handle) = stimulus.info {
                    self.base.antagonist = soldier_handle;

                    if let Some(pos_goal) =
                        self.propose_good_apple_chase_flee_destination(sim, ctx, grid)
                    {
                        self.go_to(
                            AiState::Fleeing,
                            Substate::FleeingChildChased,
                            pos_goal,
                            GotoFlags::RUN,
                            ctx,
                        );
                    } else {
                        let antag = ctx.entity_view(soldier_handle).unwrap_or_else(|| {
                            panic!(
                                "CALL_YOU_JUST_WAIT civilian {} requires chaser {} entity view",
                                self.base.me, soldier_handle
                            )
                        });
                        self.panic_from_point_at(antag.position, AI_STANDARD_PANIC_RUNS as u8);
                    }
                }
            }

            StimulusType::EventAppleChaseNear => {
                // Nearby apple chase — friend flees too
                if let StimulusInfo::Human(soldier_handle) = stimulus.info {
                    self.base.antagonist = soldier_handle;

                    if let Some(pos_goal) =
                        self.propose_good_apple_chase_flee_destination(sim, ctx, grid)
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
                        let antag = ctx.entity_view(soldier_handle).unwrap_or_else(|| {
                            panic!(
                                "EVENT_APPLE_CHASE_NEAR civilian {} requires chaser {} entity view",
                                self.base.me, soldier_handle
                            )
                        });
                        self.panic_from_point_at(antag.position, AI_STANDARD_PANIC_RUNS as u8);
                    }
                }
            }

            StimulusType::EventCouldntReachPoint => {
                if self.base.current_substate == Substate::FleeingPanic {
                    if self.base.lasting_panic_runs == 0 {
                        self.fleeing_seen_enemy_counter = 0;
                    }
                    self.base
                        .think_expected_event_common_stuff(sim, stimulus, ctx);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx);
                }
            }

            StimulusType::EventNetAway => {
                let pos = self.base.seek_position;
                self.panic_from_position(pos, AI_STANDARD_PANIC_RUNS as u8, ctx);
            }

            StimulusType::EventFitAgain => {
                // Preserve the direct Original calls around ReturnToDuty's
                // SetState callback at the engine borrow boundary.
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::InformResurrection);
                self.base
                    .outbox
                    .reentrant
                    .owner_work
                    .push(crate::ai::AiOwnerWork::SetEyeStatus(
                        crate::element::EyeStatus::LookForward,
                    ));
                let outgoing_state = self.base.current_state;
                let outgoing_substate = self.base.current_substate;
                let return_to_duty_sets_state = !ctx.in_uninterruptible_command
                    && !matches!(
                        ctx.posture,
                        crate::element::Posture::Flying
                            | crate::element::Posture::OnLadder
                            | crate::element::Posture::OnWall
                    );
                self.return_to_duty(sim, DutyFlags::empty(), ctx);
                if return_to_duty_sets_state {
                    self.base.outbox.reentrant.owner_work.push(
                        crate::ai::AiOwnerWork::StateChange(crate::ai::AiStateChangeNotification {
                            outgoing_state,
                            outgoing_substate,
                            incoming_state: self.base.current_state,
                            incoming_substate: self.base.current_substate,
                            source: crate::ai::AiStateChangeSource::SelfActor,
                            actor_effects_before_callback: Default::default(),
                        }),
                    );
                }
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
        sim: &crate::sim_rng::SimulationContext,
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
                            self.event_hear_standard_procedure(sim, &noise, ctx, grid, doors);
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
    pub fn return_to_duty(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        flags: DutyFlags,
        ctx: &AiContext,
    ) {
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
                .fire_self_stimulus(StimulusType::EventReturnToDuty);
            return;
        }

        // Call the common return-to-duty method for civilians and villains
        self.base.return_to_duty_common_stuff(sim, flags, ctx);
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
                self.base.face_position_3d_with_ctx(seek_pos, ctx);
                self.base.launch_timer(30, ctx.frame);
            }
        }
    }

    /// Standard procedure when a civilian hears something.
    pub fn event_hear_standard_procedure(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
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
                self.base.face_position_at_elevation_with_ctx(
                    noise.origin,
                    noise.elevation as f32,
                    ctx,
                );
                self.base.launch_timer(70, ctx.frame);
            }
            NoiseType::Aaargh => {
                // Scream — try to alert a soldier
                self.base.seek_position = noise.origin;

                // On a Royalist civilian's scream, the civilian
                // panics directly instead of alerting a (nearby,
                // also Royalist) soldier.
                let is_royalist = ctx.camp == crate::element::Camp::Royalists;

                if is_royalist || !self.alert_soldier(sim, noise.origin, 0, ctx, grid, doors) {
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
        self.base.face_position_3d_with_ctx(seek_pos, ctx);
        self.base.launch_timer(AI_FIRST_LOOK_TIME as u32, ctx.frame);
    }

    /// Standard procedure when a civilian sees an object.
    ///
    /// Intentionally empty — no civilian reaction is implemented.
    pub fn event_sees_object_standard_procedure(&mut self, _object: ObjectHandle) {
        // Intentionally empty.
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
        sim: &crate::sim_rng::SimulationContext,
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
        let mut detectables_to_append: Vec<(
            crate::element::EntityId,
            crate::element::DetectableType,
        )> = Vec::new();

        // RHEngine::GetSoldier(camp, i) walks the soldier registry, and
        // AddDetectable preserves that order. Hash-map iteration here made
        // the FRIEND list nondeterministic even when its membership matched.
        for &handle in ctx.all_soldier_handles.iter() {
            let view = ctx.entity_view(handle).unwrap_or_else(|| {
                panic!("alert-soldier registry handle {handle} has no live AI entity view")
            });
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
                detectables_to_append.push((
                    crate::element::EntityId::Soldier(crate::entity_id::SoldierId(handle)),
                    crate::element::DetectableType::Friend,
                ));
            }

            match view.ai_state {
                AiState::Default => {
                    // RHArtificialIntelligence::MaxNormDistance subtracts
                    // the actors' literal 3D positions, stretches world Y
                    // for the isometric projection, and only then takes the
                    // Chebyshev norm. Both `ctx.position` and `view.position`
                    // are AI planning positions that may be snapped through
                    // a door, so use the raw body positions retained beside
                    // them.
                    let my_world = ctx.self_body_position_world;
                    let dx = (view.detection_position_world.x - my_world.x).abs();
                    let dy = ((view.detection_position_world.y - my_world.y)
                        * crate::position_interface::INVERSE_ASPECT_RATIO)
                        .abs();
                    let dz = (view.detection_position_world.z - my_world.z).abs();
                    let mut distance = dx.max(dy).max(dz) as u32;

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
                            &|sector| ctx.entity_views.building_is_authorized(sector),
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
                        best = Some((
                            handle,
                            distance,
                            view.forecasted_destination.resolve(sim).position,
                        ));
                    }
                }
                AiState::Attacking | AiState::Menacing | AiState::Fleeing => {
                    // An alerted soldier is already nearby — no
                    // need to alert another.
                    if ctx.in_building || view.in_building {
                        continue;
                    }
                    let viewer_eye_z = ctx.elevation
                        + crate::stealth::eye_z_for_posture(
                            crate::element::Posture::Upright,
                            ctx.self_is_rider,
                        );
                    let viewer_eye_xy = crate::stealth::eye_point_xy(
                        crate::coordinates::MapPoint::new(ctx.position.x, ctx.position.y),
                        crate::element::Posture::Upright,
                        ctx.direction as i16,
                        false,
                    );
                    let target_detection_xy = crate::stealth::detection_point_xy(
                        crate::coordinates::MapPoint::new(view.position.x, view.position.y),
                        view.posture,
                        view.direction as i16,
                    );
                    let target_eye_z = view.elevation
                        + crate::stealth::detection_z_for_posture(view.posture, view.is_rider);
                    let viewer_eye_ground = crate::coordinates::GroundPoint::from_map_and_z(
                        viewer_eye_xy,
                        ctx.elevation,
                    );
                    let target_detection_ground = crate::coordinates::GroundPoint::from_map_and_z(
                        target_detection_xy,
                        view.elevation,
                    );
                    let dx = target_detection_ground.x - viewer_eye_ground.x;
                    let dy = (target_detection_ground.y - viewer_eye_ground.y)
                        * crate::position_interface::INVERSE_ASPECT_RATIO;
                    let dz = target_eye_z - viewer_eye_z;
                    if dx * dx + dy * dy + dz * dz <= sq_view_radius
                        && crate::sight_obstacle::is_reachable_3d(
                            ctx.obstacle_list(),
                            [viewer_eye_ground.x, viewer_eye_ground.y, viewer_eye_z],
                            [
                                target_detection_ground.x,
                                target_detection_ground.y,
                                target_eye_z,
                            ],
                            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
                        )
                    {
                        // Clear the friend list and return false.
                        // We queue the clear — the engine drains it
                        // post-think.
                        self.base
                            .outbox
                            .actor
                            .delete_detectables
                            .push(crate::element::DetectableType::Friend);
                        return false;
                    }
                }
                _ => {}
            }
        }

        // Queue the friend-detectable adds we accumulated above. Original
        // calls AddDetectable directly here: its uniqueness check is an
        // assert, so the retail build appends even when the friend is already
        // present. Keep these calls on the duplicate-preserving lane.
        // Done here (not inline) so the early-return above doesn't
        // add detectables we're about to drop.
        self.base
            .outbox
            .actor
            .append_detectables
            .extend(detectables_to_append);

        let Some((target_handle, _, target_pos)) = best else {
            // No candidate found — clear friend list and give up.
            self.base
                .outbox
                .actor
                .delete_detectables
                .push(crate::element::DetectableType::Friend);
            return false;
        };

        self.base.antagonist = target_handle;
        self.base.seek_position = center;
        self.set_state(AiState::Seeking, Substate::SeekingCivilianRunningToSoldier);
        // Run toward the picked soldier's forecasted destination
        // (e.g. the far side of an in-flight door pass) rather
        // than the animated mid-traversal position.  `target_pos`
        // is resolved from `view.forecasted_destination` at this exact
        // Original call site, so a building-exit choice owns its RNG draw.
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
                    sim,
                    center,
                    Self::ALERTFLAG_CHECK_DOOR_PATH,
                    ctx,
                    grid,
                    doors,
                );
            }
            self.base
                .outbox
                .actor
                .delete_detectables
                .push(crate::element::DetectableType::Friend);
            return false;
        }

        self.base.say(Remark::CivPanic);
        true
    }

    /// Random ambient speech for civilians.
    ///
    /// Called each frame; only acts every 256 frames (`frame_phase == 0`).
    pub fn random_speech(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        frame_phase: u8,
        ctx: &AiContext,
    ) {
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
                && crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CivilianBeggarSpeechGate, 0..3)
                    == 0
            {
                match crate::sim_rng::u32(
                    sim,
                    crate::sim_rng::RngSite::CivilianBeggarSpeechChoice,
                    0..5,
                ) {
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
        ctx: &AiContext,
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
                        if dest.sector.is_some() {
                            let flags = self.base.last_goto_flags;
                            // Original retries through GoTo itself, including
                            // its synchronous already-on-point callback.
                            self.base.go_to(dest, flags, ctx);
                        } else {
                            self.base
                                .outbox
                                .reentrant
                                .self_stimuli
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
    pub fn init_one_ai(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
    ) -> InitStateSideEffects {
        // Default civilian life points are set on `NpcData::default()`
        // (in `element.rs`) to the engine's `CIVILIAN_LIFE_POINTS = 100`.

        // `go_to_duty = init_state(sim, ) && !ai_is_script_locked() && !ai_is_locked()`.
        // The `init_state` call commits the AI-side state
        // transition chosen by the level designer's authored
        // initial action and tells us whether the actor should
        // launch into its duty loop after.
        let fx = self.base.init_state(sim, ctx);

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
                self.return_to_duty(sim, DutyFlags::empty(), ctx);
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
            // A failed GoTo and a no-op FaceTo raise their latches
            // unconditionally, with no outside-Think delivery path of their
            // own. Outside a Think the next Think entry simply discards them,
            // so drop them here instead of inventing completions.
            self.base.couldnt_reachpoint = false;
            self.base.already_turned = false;
        } else if go_to_duty {
            // Civilians without a patrol path and `go_to_duty=true`
            // get the authored "first look" randomised delay.
            // `init_state` already launched the bored timer via
            // its `WaitingUpright` branch, so we overwrite with the
            // longer look timer here — the second `launch_timer`
            // call wins.
            let timer_value = AB_MIN_DEFAULT_LOOK_TIME
                + crate::sim_rng::i32(
                    sim,
                    crate::sim_rng::RngSite::CivilianFirstLookTimer,
                    0..AB_DELTA_DEFAULT_LOOK_TIME,
                );
            self.base.launch_timer(timer_value as u32, ctx.frame);
            self.set_state(AiState::Default, Substate::DefaultOnPost);
            self.base.substate_at_last_timer_launch = self.base.current_substate;
        }

        // Original InitOneAI stamps this after all patrol-path setup.
        self.base.last_hint_actuality = ctx.frame;

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
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) -> Option<Position> {
        let antagonist = ctx.entity_view(self.base.antagonist)?;

        // Base direction is antagonist→self, jittered by
        // `(rand()%5) + 14` which is `-2..+2` mod 16.
        let dx = ctx.position.x - antagonist.position.x;
        let dy = ctx.position.y - antagonist.position.y;
        let base_dir = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy) as i32;
        let jitter =
            crate::sim_rng::u32(sim, crate::sim_rng::RngSite::CivilianPanicDirection, 0..5) as i32;
        let seed_dir = (base_dir + jitter + 14).rem_euclid(16);

        // Relative direction sequence:
        // 0, 1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6, -6, 7, -7.
        let rel_sequence: [i32; 15] = [0, 1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6, -6, 7, -7];

        let origin_pt = MapPoint::new(ctx.position.x, ctx.position.y);

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
                        MapPoint::new(dest.x, dest.y),
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
    fn civilian_start_think_distinguishes_static_and_ailock_freeze() {
        let ctx = AiContext::default();
        let stimulus = Stimulus::new(StimulusType::EventTimer);

        let mut static_frozen = FriendlyAi::new(1);
        assert!(!static_frozen.start_think(&stimulus, &ctx, true));
        assert!(static_frozen.base.stimulus_queue.is_empty());

        let mut ai_locked = FriendlyAi::new(2);
        ai_locked.base.locks_flag_field = AiLockFlags::FREEZE;
        assert!(!ai_locked.start_think(&stimulus, &ctx, false));
        assert_eq!(ai_locked.base.stimulus_queue.len(), 1);
        assert_eq!(
            ai_locked.base.stimulus_queue[0].stimulus_type,
            StimulusType::EventTimer
        );
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
    fn patrol_coordinate_uses_friendly_virtual_state_before_walk_and_run() {
        for (distance, expected_substate, expected_order) in [
            (
                45.0,
                Substate::DefaultPatrolEnroute,
                crate::order::OrderType::WalkingUpright,
            ),
            (
                60.0,
                Substate::DefaultPatrolEnrouteRunning,
                crate::order::OrderType::RunningUpright,
            ),
        ] {
            let mut ai = FriendlyAi::new(1);
            ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
                crate::entity_id::SoldierId(2),
            ));
            ai.base.current_state = AiState::Default;
            ai.base.current_substate = Substate::DefaultOnPost;
            ai.base.current_music_alert_status = AlertLevel::Yellow;
            ai.base.view_alert_status = AlertLevel::Yellow;

            let ctx = AiContext {
                position: Position {
                    x: 100.0,
                    y: 100.0,
                    sector: SectorHandle::new(1),
                    level: 0,
                },
                ..AiContext::default()
            };
            let target = Position {
                x: ctx.position.x + distance,
                ..ctx.position
            };
            ai.coordinate_patrol(
                &StimulusInfo::Position(target),
                &ctx,
                Position {
                    x: ctx.position.x + 100.0,
                    ..ctx.position
                },
            );

            assert_eq!(ai.base.current_state, AiState::Default);
            assert_eq!(ai.base.current_substate, expected_substate);
            assert_eq!(ai.base.current_music_alert_status, AlertLevel::Green);
            assert_eq!(ai.base.view_alert_status, AlertLevel::Green);
            let [AiOwnerWork::StateChange(notification)] =
                ai.base.outbox.reentrant.owner_work.as_slice()
            else {
                panic!("patrol coordinate must call Bonhomie's virtual SetState");
            };
            let prefix = notification
                .actor_effects_before_callback
                .as_ref()
                .expect("StopAll must precede the friendly state callback");
            assert!(prefix.halt);
            let [order] = ai.base.outbox.actor.orders.as_slice() else {
                panic!("patrol coordinate must queue one replacement movement");
            };
            assert_eq!(order.order_type, expected_order);
        }
    }

    #[test]
    fn patrol_coordinate_same_substate_still_calls_friendly_state_without_stop_prefix() {
        let mut ai = FriendlyAi::new(1);
        ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
            crate::entity_id::SoldierId(2),
        ));
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultPatrolEnroute;
        ai.base.current_music_alert_status = AlertLevel::Yellow;
        ai.base.view_alert_status = AlertLevel::Yellow;
        let ctx = AiContext {
            position: Position {
                x: 100.0,
                y: 100.0,
                sector: SectorHandle::new(1),
                level: 0,
            },
            ..AiContext::default()
        };

        ai.coordinate_patrol(
            &StimulusInfo::Position(Position {
                x: 145.0,
                ..ctx.position
            }),
            &ctx,
            Position {
                x: 200.0,
                ..ctx.position
            },
        );

        assert_eq!(ai.base.current_music_alert_status, AlertLevel::Green);
        assert_eq!(ai.base.view_alert_status, AlertLevel::Green);
        let [AiOwnerWork::StateChange(notification)] =
            ai.base.outbox.reentrant.owner_work.as_slice()
        else {
            panic!("Bonhomie SetState must notify even when the substate is unchanged");
        };
        assert!(notification.actor_effects_before_callback.is_none());
        let [order] = ai.base.outbox.actor.orders.as_slice() else {
            panic!("same-substate patrol update must queue its movement");
        };
        assert_eq!(order.order_type, crate::order::OrderType::WalkingUpright);
    }

    #[test]
    #[should_panic(expected = "civilian 1 running to required antagonist 42 has no entity view")]
    fn review_running_to_soldier_requires_the_live_antagonist_view() {
        let sim = crate::sim_rng::test_context();
        let mut ai = FriendlyAi::new(1);
        ai.base.antagonist = 42;
        ai.set_state(AiState::Seeking, Substate::SeekingCivilianRunningToSoldier);
        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventReachPoint),
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &FriendlyPerTickData::without_patrol_chief(),
            None,
            None,
        );
    }

    #[test]
    fn civilian_return_to_duty() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        ai.fleeing_seen_enemy_counter = 5;
        ai.set_state(AiState::Fleeing, Substate::FleeingPanic);
        ai.return_to_duty(sim, DutyFlags::empty(), &AiContext::default());
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
    #[should_panic(expected = "CALL_YOU_JUST_WAIT civilian 1 requires chaser 42 entity view")]
    fn apple_chase_does_not_replace_a_missing_chaser_with_undirected_panic() {
        let sim = crate::sim_rng::test_context();
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.think_unexpected_event(
            &sim,
            &Stimulus::with_human(StimulusType::CallYouJustWait, 42),
            &mut global,
            &AiContext::default(),
            &FriendlyPerTickData::without_patrol_chief(),
            None,
            None,
        );
    }

    #[test]
    fn think_expected_admiring_hero_returns_to_duty() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(AiState::Wondering, Substate::WonderingCivilianAdmiringHero);

        let stimulus = Stimulus::new(StimulusType::EventTimer);
        ai.think_expected_event(
            sim,
            &stimulus,
            &mut global,
            &AiContext::default(),
            &FriendlyPerTickData::without_patrol_chief(),
            None,
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Default);
        // Walks back to post first (DefaultGotoPost → EventReachPoint → OnPost).
        assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
    }

    #[test]
    fn think_alerting_event_panic() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let pos = Position {
            x: 50.0,
            y: 75.0,
            sector: None,
            level: 0,
        };
        let stimulus = Stimulus::with_position(StimulusType::EventPanic, pos);

        ai.think_alerting_event(sim, &stimulus, &AiContext::default(), None, None);

        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        assert_eq!(ai.base.panic_center_x, 50.0);
        assert_eq!(ai.base.panic_center_y, 75.0);
    }

    #[test]
    fn repeated_event_panic_preserves_red_alert_until_the_panic_drain() {
        let sim = crate::sim_rng::test_context();
        let mut ai = FriendlyAi::new(1);
        ai.set_state(AiState::Fleeing, Substate::FleeingPanic);
        ai.base.set_alert_status(AlertLevel::Red);
        ai.base.outbox.reentrant.owner_work.clear();

        let panic_center = Position {
            x: 50.0,
            y: 75.0,
            sector: None,
            level: 0,
        };
        ai.think_alerting_event(
            &sim,
            &Stimulus::with_position(StimulusType::EventPanic, panic_center),
            &AiContext::default(),
            None,
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        assert_eq!(ai.base.current_music_alert_status, AlertLevel::Red);
        assert_eq!(ai.base.view_alert_status, AlertLevel::Red);
        assert!(ai.base.outbox.reentrant.owner_work.is_empty());
        let request = ai
            .base
            .outbox
            .actor
            .begin_panic
            .expect("repeated EVENT_PANIC must still reach the synchronous panic drain");
        assert_eq!(request.center, Some(panic_center));
        assert!(!request.is_new_panic);
    }

    #[test]
    fn think_alerting_event_stop() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let stimulus = Stimulus::new(StimulusType::EventStop);

        ai.think_alerting_event(sim, &stimulus, &AiContext::default(), None, None);

        assert_eq!(ai.base.current_state, AiState::Seeking);
        assert_eq!(ai.base.current_substate, Substate::SeekingGotStopEvent);
        assert!(ai.base.timer_is_running);
    }

    #[test]
    fn think_alerting_event_stop_while_sleeping() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        ai.set_state(AiState::Sleeping, Substate::SleepingForever);
        let stimulus = Stimulus::new(StimulusType::EventStop);

        let result = ai.think_alerting_event(sim, &stimulus, &AiContext::default(), None, None);

        // Should return false and NOT change state
        assert!(!result);
        assert_eq!(ai.base.current_state, AiState::Sleeping);
    }

    #[test]
    fn think_unexpected_couldnt_reachpoint_returns_to_duty() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(AiState::Seeking, Substate::SeekingCivilianRunningToSoldier);

        let stimulus = Stimulus::new(StimulusType::EventCouldntReachPoint);
        ai.think_unexpected_event(
            sim,
            &stimulus,
            &mut global,
            &AiContext::default(),
            &FriendlyPerTickData::without_patrol_chief(),
            None,
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Default);
        // Walks back to post first.
        assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
    }

    #[test]
    fn after_script_queue_rebuilds_retained_view_antagonist() {
        use crate::ai_entity_view::{AiEntityViewMap, EntityKind, shared_entity_views};
        use crate::element::Camp;

        let sim = crate::sim_rng::test_context();
        let mut global = AiGlobalState::default();
        let mut ai = FriendlyAi::new(1);
        let target = 42;
        let target_pos = Position {
            x: 150.0,
            y: 250.0,
            sector: None,
            level: 0,
        };
        let mut target_view = make_soldier_view(target_pos, Camp::Lacklandists, AiState::Attacking);
        target_view.kind = EntityKind::Pc;
        target_view.is_pc = true;
        target_view.is_swordfighting = true;
        let mut views = AiEntityViewMap::new();
        views.insert(target, target_view);
        let ctx = AiContext {
            camp: Camp::Royalists,
            entity_views: shared_entity_views(views),
            // The outer EVENT_AFTER_SCRIPT_GO_ON has no antagonist.
            antagonist: None,
            ..AiContext::default()
        };

        ai.base
            .stimulus_queue
            .push(Stimulus::with_human(StimulusType::EventView, target));
        ai.think_unexpected_event(
            &sim,
            &Stimulus::new(StimulusType::EventAfterScriptGoOn),
            &mut global,
            &ctx,
            &FriendlyPerTickData::without_patrol_chief(),
            None,
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        let request = ai
            .base
            .outbox
            .actor
            .begin_panic
            .expect("retained swordfighter view must launch panic");
        assert_eq!(request.center, Some(target_pos));
    }

    #[test]
    fn think_unexpected_fit_again_returns_to_duty() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(AiState::Sleeping, Substate::SleepingUnconscious);

        let stimulus = Stimulus::new(StimulusType::EventFitAgain);
        ai.think_unexpected_event(
            sim,
            &stimulus,
            &mut global,
            &AiContext::default(),
            &FriendlyPerTickData::without_patrol_chief(),
            None,
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Default);
    }

    #[test]
    fn fit_again_queues_ordered_resurrection_eye_and_state_work() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        // EVENT_FITAGAIN must fire the resurrection fan-out and
        // reset the view status to LookForward alongside the
        // return-to-duty hand-off. They share the owner FIFO with
        // ReturnToDuty's state callback so the engine can preserve the
        // Original statement boundary.
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(AiState::Sleeping, Substate::SleepingUnconscious);
        ai.base.outbox.reentrant.owner_work.clear();

        let stimulus = Stimulus::new(StimulusType::EventFitAgain);
        ai.think_unexpected_event(
            sim,
            &stimulus,
            &mut global,
            &AiContext::default(),
            &FriendlyPerTickData::without_patrol_chief(),
            None,
            None,
        );

        assert!(matches!(
            ai.base.outbox.reentrant.owner_work.as_slice(),
            [
                crate::ai::AiOwnerWork::InformResurrection,
                crate::ai::AiOwnerWork::SetEyeStatus(crate::element::EyeStatus::LookForward),
                crate::ai::AiOwnerWork::StateChange(_),
            ]
        ));
        assert!(!ai.base.outbox.recovery.inform_resurrection);
        assert_eq!(ai.base.outbox.recovery.set_eye_status, None);
    }

    #[test]
    fn fleeing_event_view_uses_directed_panic_from_human_position() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
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
                original_creation_order: 41,
                position: enemy_pos,
                detection_position: MapPoint::new(enemy_pos.x, enemy_pos.y),
                detection_position_world: crate::coordinates::WorldPoint3D::new(
                    enemy_pos.x,
                    enemy_pos.y,
                    0.0,
                ),
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
                active: true,
                is_unconscious: false,
                action_state: crate::element::ActionState::Waiting,
                passing_door: false,
                obstacle_idx: None,
                in_building: false,
                building_sector: None,
                ai_state: AiState::Default,
                ai_substate: Substate::DefaultOnPost,
                script_locked: false,
                forecasted_destination: crate::ai::PreparedForecastDestination::fixed(enemy_pos, 0),
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
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let stimulus = Stimulus::with_human(StimulusType::EventView, human_handle);
        ai.think_alerting_event(sim, &stimulus, &ctx, None, None);

        assert!(
            ai.base.directed_panic,
            "EVENT_VIEW while fleeing must fire a *directed* panic"
        );
        let request = ai
            .base
            .outbox
            .actor
            .begin_panic
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
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        let stimulus = Stimulus::new(StimulusType::EventNetAway);

        ai.think_unexpected_event(
            sim,
            &stimulus,
            &mut global,
            &AiContext::default(),
            &FriendlyPerTickData::without_patrol_chief(),
            None,
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    }

    #[test]
    fn patrol_coordinate_uses_real_chief_position_for_near_backwards_gate() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
            crate::entity_id::SoldierId(2),
        ));
        ai.set_state(AiState::Default, Substate::DefaultPatrolEnroute);

        let ctx = AiContext {
            position: Position {
                x: 0.0,
                y: 0.0,
                sector: SectorHandle::new(1),
                level: 0,
            },
            direction: 0,
            ..AiContext::default()
        };
        let tick = FriendlyPerTickData::with_patrol_chief(
            Position {
                x: 100.0,
                y: 0.0,
                ..ctx.position
            },
            AiState::Default,
        );
        let stimulus = Stimulus::with_position(
            StimulusType::CallPatrolCoordinate,
            Position {
                x: -10.0,
                y: 0.0,
                ..ctx.position
            },
        );

        ai.think_unexpected_event(sim, &stimulus, &mut global, &ctx, &tick, None, None);
        let orders = ai.base.take_pending_orders();

        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].order_type, crate::order::OrderType::Turning);
        assert!(
            !ai.base.already_on_point,
            "near-backwards patrol coordinate must turn toward the chief, not walk to the slot"
        );
    }

    #[test]
    #[should_panic(expected = "requires a live patrol-chief snapshot")]
    fn patrol_handler_cannot_silently_consume_missing_friendly_tick_data() {
        let sim = crate::sim_rng::test_context();
        let mut global = AiGlobalState::default();
        let mut ai = FriendlyAi::new(1);
        ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
            crate::entity_id::SoldierId(2),
        ));
        ai.set_state(AiState::Default, Substate::DefaultPatrolEnrouteWaiting);
        ai.think_expected_event(
            &sim,
            &Stimulus::new(StimulusType::EventTimer),
            &mut global,
            &AiContext::default(),
            &FriendlyPerTickData::without_patrol_chief(),
            None,
            None,
        );
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
        assert!(ai.base.outbox.reentrant.owner_work.iter().any(|work| {
            matches!(
                work,
                AiOwnerWork::Speech(AiSpeechAttempt {
                    remark: Remark::CivSeesBody,
                    flags: 0,
                })
            )
        }));
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
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(
            AiState::Wondering,
            Substate::WonderingCivilianBodyReactiontime,
        );

        let stimulus = Stimulus::new(StimulusType::EventTimer);
        ai.think_expected_event(
            sim,
            &stimulus,
            &mut global,
            &AiContext::default(),
            &FriendlyPerTickData::without_patrol_chief(),
            None,
            None,
        );

        // AlertSoldier returns false (stub) → should panic
        assert_eq!(ai.base.current_state, AiState::Fleeing);
        assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
        assert!(ai.base.outbox.reentrant.owner_work.iter().any(|work| {
            matches!(
                work,
                AiOwnerWork::Speech(AiSpeechAttempt {
                    remark: Remark::CivPanic,
                    flags: 0,
                })
            )
        }));
    }

    #[test]
    fn expected_event_whistling_child_approaches() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(AiState::Wondering, Substate::WonderingWatchingWhistling);

        let stimulus = Stimulus::new(StimulusType::EventTimer);
        ai.think_expected_event(
            sim,
            &stimulus,
            &mut global,
            &AiContext::default(),
            &FriendlyPerTickData::without_patrol_chief(),
            None,
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Wondering);
        assert_eq!(
            ai.base.current_substate,
            Substate::WonderingChildApproachingWhistling,
        );
        assert!(ai.base.outbox.reentrant.owner_work.iter().any(|work| {
            matches!(
                work,
                AiOwnerWork::Speech(AiSpeechAttempt {
                    remark: Remark::CivWhistling,
                    flags: 0,
                })
            )
        }));
    }

    #[test]
    fn fleeing_child_chased_end_returns_to_duty() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(AiState::Fleeing, Substate::FleeingChildChasedEnd);

        let stimulus = Stimulus::new(StimulusType::EventTimer);
        ai.think_expected_event(
            sim,
            &stimulus,
            &mut global,
            &AiContext::default(),
            &FriendlyPerTickData::without_patrol_chief(),
            None,
            None,
        );

        assert_eq!(ai.base.current_state, AiState::Default);
    }

    #[test]
    fn seeking_report_point_done_transitions() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let mut global = AiGlobalState::default();
        ai.set_state(
            AiState::Seeking,
            Substate::SeekingCivilianGiveAlertingReportToSoldierPoint,
        );

        let stimulus = Stimulus::new(StimulusType::EventDone);
        ai.think_expected_event(
            sim,
            &stimulus,
            &mut global,
            &AiContext::default(),
            &FriendlyPerTickData::without_patrol_chief(),
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
            original_creation_order: 41,
            position: pos,
            detection_position: MapPoint::new(pos.x, pos.y),
            detection_position_world: crate::coordinates::WorldPoint3D::new(pos.x, pos.y, 0.0),
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
            active: true,
            is_unconscious: false,
            action_state: crate::element::ActionState::Waiting,
            passing_door: false,
            obstacle_idx: None,
            in_building: false,
            building_sector: None,
            ai_state,
            ai_substate: Substate::DefaultOnPost,
            script_locked: false,
            forecasted_destination: crate::ai::PreparedForecastDestination::fixed(pos, 0),
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
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
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
            sq_self_view_radius: 1_000_000.0,
            all_soldier_handles: std::sync::Arc::new(vec![10, 20]),
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let ok = ai.alert_soldier(sim, ctx.position, 0, &ctx, None, None);
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
            sq_self_view_radius: 1.0,
            all_soldier_handles: std::sync::Arc::new(vec![10, 20]),
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        crate::sim_rng::with_seed(1, |sim| {
            let ok = ai.alert_soldier(sim, ctx.position, 0, &ctx, None, None);
            assert!(ok, "alert_soldier must succeed when at least one candidate");
            // Antagonist must be the same-layer one despite being farther.
            assert_eq!(ai.base.antagonist, 20);
        });
    }

    #[test]
    fn alert_soldier_ranks_with_stretched_world_max_norm() {
        use crate::ai_entity_view::AiEntityViewMap;
        use crate::element::Camp;

        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let owner = Position {
            x: 1680.0,
            y: 2065.0,
            sector: crate::position_interface::SectorHandle::new(0),
            level: 0,
        };
        // Raw projected map distance makes handle 141 look nearer:
        // max(105, 379) < max(414, 213). Original stretches world Y,
        // yielding 660 for handle 141 but only 414 for handle 130.
        let mut views = AiEntityViewMap::new();
        views.insert(
            141,
            make_soldier_view(
                Position {
                    x: 1785.0,
                    y: 1686.0,
                    sector: crate::position_interface::SectorHandle::new(0),
                    level: 0,
                },
                Camp::Lacklandists,
                AiState::Default,
            ),
        );
        views.insert(
            130,
            make_soldier_view(
                Position {
                    x: 1266.0,
                    y: 2278.0,
                    sector: crate::position_interface::SectorHandle::new(18),
                    level: 0,
                },
                Camp::Lacklandists,
                AiState::Default,
            ),
        );
        let ctx = AiContext {
            position: owner,
            self_body_position_world: crate::coordinates::WorldPoint3D::new(owner.x, owner.y, 0.0),
            camp: Camp::Lacklandists,
            all_soldier_handles: std::sync::Arc::new(vec![130, 141]),
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        assert!(ai.alert_soldier(sim, owner, 0, &ctx, None, None));
        assert_eq!(ai.base.antagonist, 130);
    }

    #[test]
    fn alert_soldier_ranks_from_raw_body_when_planning_position_is_gate_snapped() {
        use crate::ai_entity_view::AiEntityViewMap;
        use crate::coordinates::WorldPoint3D;
        use crate::element::Camp;

        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut ai = FriendlyAi::new(1);
        let mut views = AiEntityViewMap::new();

        let mut near_raw = make_soldier_view(
            Position {
                x: 300.0,
                y: 0.0,
                ..Position::default()
            },
            Camp::Lacklandists,
            AiState::Default,
        );
        near_raw.detection_position_world = WorldPoint3D::new(300.0, 0.0, 500.0);
        views.insert(10, near_raw);

        let mut near_gate = make_soldier_view(
            Position {
                x: 900.0,
                y: 0.0,
                ..Position::default()
            },
            Camp::Lacklandists,
            AiState::Default,
        );
        near_gate.detection_position_world = WorldPoint3D::new(900.0, 0.0, 100.0);
        views.insert(20, near_gate);

        let mut near_only_if_z_is_ignored = make_soldier_view(
            Position {
                x: 200.0,
                y: 0.0,
                ..Position::default()
            },
            Camp::Lacklandists,
            AiState::Default,
        );
        near_only_if_z_is_ignored.detection_position_world = WorldPoint3D::new(200.0, 0.0, 1000.0);
        views.insert(30, near_only_if_z_is_ignored);

        let ctx = AiContext {
            // AI Position() has already committed to the far gate endpoint,
            // while GetPosition() still reports the interpolating body.
            position: Position {
                x: 1000.0,
                y: 0.0,
                ..Position::default()
            },
            self_body_position_world: WorldPoint3D::new(0.0, 0.0, 100.0),
            camp: Camp::Lacklandists,
            all_soldier_handles: std::sync::Arc::new(vec![10, 20, 30]),
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        assert!(ai.alert_soldier(sim, ctx.position, 0, &ctx, None, None));
        // Raw 3D MaxNorm distances are 400, 900, and 900. Reconstructing the
        // owner from the gate-snapped planning position would choose 20;
        // dropping the nonzero Z component while retaining raw X would choose
        // 30. Original's literal raw 3D operation must instead choose 10.
        assert_eq!(ai.base.antagonist, 10);
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
            20,
            make_soldier_view(
                Position {
                    x: 200.0,
                    y: 0.0,
                    sector: None,
                    level: 0,
                },
                Camp::Royalists,
                AiState::Default,
            ),
        );
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
            sq_self_view_radius: 1.0,
            // Deliberately opposite the insertion order above: the Original
            // observes registry order, not HashMap bucket order.
            all_soldier_handles: std::sync::Arc::new(vec![20, 10]),
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        crate::sim_rng::with_seed(1, |sim| {
            ai.alert_soldier(sim, ctx.position, 0, &ctx, None, None);
            let notification = ai
                .base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .find_map(|work| match work {
                    AiOwnerWork::StateChange(notification) => Some(notification),
                    _ => None,
                })
                .expect("alerting a soldier must enter Seeking through Friendly SetState");
            let effects = notification
                .actor_effects_before_callback
                .as_ref()
                .expect("friend detectables must precede the Friendly state callback");
            let friends: Vec<_> = effects
                .append_detectables
                .iter()
                .filter(|(_, t)| *t == DetectableType::Friend)
                .map(|(entity, _)| entity.index())
                .collect();
            assert_eq!(
                friends,
                vec![20, 10],
                "friend detectables must preserve the soldier registry order"
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
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        crate::sim_rng::with_seed(1, |sim| {
            let dest = ai.propose_good_apple_chase_flee_destination(sim, &ctx, None);
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
