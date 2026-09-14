//! Friendly (civilian) AI.
//!
//! This module contains the `FriendlyAi` struct which extends [`AiController`]
//! with civilian-specific state: talking, panic behavior, beggar interactions,
//! and the civilian Think state machine.

use serde::{Deserialize, Serialize};

use crate::ai::*;
use crate::coordinates::MapPoint;
use crate::parameters_ai::{AI_FIRST_LOOK_TIME, AI_STANDARD_PANIC_RUNS};

// ---------------------------------------------------------------------------
// Civilian-specific constants
// ---------------------------------------------------------------------------

pub const APPLE_CHASE_IDEAL_DISTANCE: i32 = 300;
pub const BEGGAR_NO_RANDOM_TALK_DISTANCE: i32 = 100;

// ---------------------------------------------------------------------------
// FriendlyAi — extends AiController with civilian-specific state
// ---------------------------------------------------------------------------

/// Civilian AI state. Extends [`AiController`] with civilian-specific fields.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct FriendlyAi {
    /// Base AI controller (contains all common state).
    pub base: AiController,

    // -- Civilian-specific private fields --
    pub beggar_dont_talk_counter: u16,
    pub fleeing_seen_enemy_counter: u16,
    /// Whether this civilian still accepts the talk interaction.
    pub wants_to_talk: bool,
    /// Last NPC this civilian talked to. The original game initializes this reference to
    /// null and preserves that nullability in saved games.
    // TODO: historically persisted untagged (bare handle), unlike the tagged
    // `optional_ai_handle` fields; kept for save compatibility.
    pub last_talk_partner: Option<AiEntityHandle>,
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
            last_talk_partner: None,
            can_go_away: true,
        }
    }
}

impl AiRole for FriendlyAi {
    fn base_mut(&mut self) -> &mut AiController {
        &mut self.base
    }

    #[track_caller]
    fn role_set_state(&mut self, state: AiState, substate: Substate) {
        FriendlyAi::set_state(self, state, substate);
    }

    /// Civilians have no view override; use the base alert setter.
    fn role_set_alert_status(&mut self, level: AlertLevel) {
        self.base.set_alert_status(level);
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
    pub(crate) fn begin_state_change(&mut self, state: AiState, substate: Substate) {
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
    }

    pub fn set_state(&mut self, state: AiState, substate: Substate) {
        self.begin_state_change(state, substate);

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
        // Work done before changing state belongs inside its synchronous
        // callback boundary. In particular, common patrol coordination falls
        // through from stop-all, so its halt must be applied before the
        // civilian FilterAIEvent while the following movement remains outside.
        let actor_effects_before_callback = self
            .base
            .outbox
            .actor
            .has_boundary_work()
            .then(|| std::mem::take(&mut self.base.outbox.actor));
        self.base
            .queue_state_change(state, substate, source, actor_effects_before_callback);

        self.base.set_ai_state(state);
        self.base.current_substate = substate;
    }

    // Movement helpers (`go_to`, `go_to_speed`, `go_near`) and
    // `coordinate_patrol` are shared with the enemy role via [`AiRole`].

    // Panic requests resume against live engine state after releasing this borrow.

    /// Panic fleeing from a specific point, tagged with the sector
    /// and level of its origin so the engine's door lookup can
    /// resolve multi-level flee paths correctly.
    pub(crate) fn panic_from_point_at(&mut self, center: Position, runs: u8) {
        // Capture the "new panic" flag before the state transition:
        // the drain's no-door arm uses this to suppress repeated
        // State-change / speech / reach-point self-fires when we're already
        // in panic.
        let was_already_fleeing = matches!(
            self.base.current_substate,
            Substate::FleeingPanic | Substate::FleeingRunToDoor
        );
        self.base.panic_center_x = center.x;
        self.base.panic_center_y = center.y;
        self.base.directed_panic = true;
        self.base.outbox.actor.begin_panic = Some(PanicRequest {
            center: Some(center),
            runs,
            alert: AlertLevel::Red,
            is_new_panic: !was_already_fleeing,
        });
    }

    /// Undirected panic.
    pub(crate) fn panic_undirected(&mut self, runs: u8) {
        let was_already_fleeing = matches!(
            self.base.current_substate,
            Substate::FleeingPanic | Substate::FleeingRunToDoor
        );
        self.base.directed_panic = false;
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

    /// Admit a civilian Think without retaining its borrow across callbacks.
    /// The engine completes both admitted and rejected calls after releasing this borrow.
    pub(crate) fn begin_think(
        &mut self,
        _sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiAdmission,
    ) -> bool {
        self.base.cached_frame = ctx.frame;
        self.base.cached_in_building = ctx.in_building;

        let stimulus_type = stimulus.stimulus_type;

        // Pre-think checks
        if !self.start_think(stimulus, ctx, global.freeze) {
            if stimulus_type == StimulusType::EventAfterScriptGoOn {
                self.base.outbox.reentrant.engine_drains_after_script_go_on = false;
            }
            return false;
        }

        // Script filter gate applied by the engine before this call —
        // see `Engine::filter_stimulus` and the matching note in
        // the engine-owned decision dispatcher.

        true
    }

    /// Run only the admitted handler; the engine owns the surrounding call.
    pub(crate) fn think_body(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        doors: Option<&[crate::gate::Door]>,
    ) -> AiFlow<bool> {
        let stimulus_type = stimulus.stimulus_type;

        Ok(match stimulus_type {
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
                self.think_expected_event(sim, stimulus, ctx, grid, doors)?
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
                self.think_unexpected_event(sim, stimulus, global, ctx, grid, doors)?
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
                return Err(DutyCall::new(DutyFlags::empty(), false));
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
        })
    }

    // -----------------------------------------------------------------------
    // Think sub-methods
    // -----------------------------------------------------------------------

    fn start_think(
        &mut self,
        stimulus: &Stimulus,
        ctx: &AiAdmission,
        static_ai_frozen: bool,
    ) -> bool {
        self.start_think_post_filter(stimulus, ctx, static_ai_frozen)
    }

    // `start_think_pre_filter` (decision-tick admission before the script
    // `FilterAIEvent` call) is shared with the enemy role via [`AiRole`].

    /// Decision-tick admission work after `FilterAIEvent`. SetAIState observes these
    /// gates but deliberately ignores the returned admission decision.
    /// Civilians normally never hit `EventWasp` / `EventNet`, but the gates
    /// live on the shared behavior so any scripted substate change could
    /// reach them; mirror the enemy path's defensive refusals.
    pub(crate) fn start_think_post_filter(
        &mut self,
        stimulus: &Stimulus,
        ctx: &AiAdmission,
        static_ai_frozen: bool,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;

        if !self
            .base
            .admit_think_before_role_gates(stimulus, static_ai_frozen)
        {
            return false;
        }

        if !self.base.admit_think_after_role_gates(stimulus, ctx) {
            return false;
        }

        // These three stimuli are consumed by the common
        // AI think-start behavior before the
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

    // -----------------------------------------------------------------------
    // Expected-event civilian dispatcher
    // -----------------------------------------------------------------------

    fn think_expected_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        _doors: Option<&[crate::gate::Door]>,
    ) -> AiFlow<bool> {
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

            Substate::DefaultChildApproachedWhistling => {
                if stimulus_type == StimulusType::EventTimer {
                    return Err(DutyCall::new(DutyFlags::empty(), false));
                }
            }

            // ############## W O N D E R I N G #####################
            Substate::WonderingCivilianAdmiringHero => {
                if stimulus_type == StimulusType::EventTimer {
                    return Err(DutyCall::new(DutyFlags::empty(), false));
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
            Substate::SeekingGotStopEvent => {
                if stimulus_type == StimulusType::EventTimer {
                    return Err(DutyCall::new(DutyFlags::empty(), false));
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
                            self.panic_from_point_at(panic_center, AI_STANDARD_PANIC_RUNS as u8);
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
                    return Err(DutyCall::new(DutyFlags::empty(), false));
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
        Ok(false)
    }

    // -----------------------------------------------------------------------
    // Unexpected-event civilian dispatcher
    // -----------------------------------------------------------------------

    pub(crate) fn think_unexpected_event(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        _doors: Option<&[crate::gate::Door]>,
    ) -> AiFlow<bool> {
        let stimulus_type = stimulus.stimulus_type;

        match stimulus_type {
            StimulusType::EventAfterScriptGoOn => {
                if self.base.outbox.reentrant.engine_drains_after_script_go_on {
                    return Ok(false);
                }
                // The engine drains retained stimuli before invoking this tail.
                // After the drain, if we're in STATE_DEFAULT we
                // either advance on the patrol path (next waypoint
                // → enter the en-route state → move) or return
                // to duty. Outside STATE_DEFAULT we leave
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
                            .and_then(|p| {
                                p.current_waypoint(hiking_paths)
                                    .map(|wp| (p.hiking_path_index, p.current_waypoint_index, wp))
                            })
                            .map(|(path_index, waypoint_index, wp)| {
                                (
                                    Position {
                                        x: wp.x as f32,
                                        y: wp.y as f32,
                                        sector: ctx.hiking_waypoint_sector(
                                            usize::from(path_index),
                                            usize::from(waypoint_index),
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
                            return Err(DutyCall::new(DutyFlags::empty(), false));
                        }
                    } else {
                        return Err(DutyCall::new(DutyFlags::empty(), false));
                    }
                    return Ok(false);
                }
            }

            StimulusType::CallYouJustWait => {
                // Soldier tells child to wait (apple chase begins)
                if let StimulusInfo::Human(soldier_handle) = stimulus.info {
                    self.base.antagonist = Some(soldier_handle);

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
                        let antag = ctx.entity_view(soldier_handle.get()).unwrap_or_else(|| {
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
                    self.base.antagonist = Some(soldier_handle);

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
                        let antag = ctx.entity_view(soldier_handle.get()).unwrap_or_else(|| {
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
                        .think_expected_event_common_stuff(sim, stimulus, ctx)?;
                } else {
                    return Err(DutyCall::new(DutyFlags::empty(), false));
                }
            }

            StimulusType::EventNetAway => {
                let pos = self.base.seek_position;
                self.panic_from_point_at(pos, AI_STANDARD_PANIC_RUNS as u8);
            }

            StimulusType::EventFitAgain => {
                // Preserve the original game's direct work around return-to-duty
                // state-change callback at the engine borrow boundary.
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
                return Err(DutyCall::new(DutyFlags::empty(), false));
            }

            StimulusType::EventOutOfView => {
                // Lost sight of someone — civilians don't react
            }

            _ => {}
        }

        Ok(false)
    }

    // -----------------------------------------------------------------------
    // Alerting-event civilian dispatcher
    // -----------------------------------------------------------------------

    fn think_alerting_event(
        &mut self,
        _sim: &crate::sim_rng::SimulationContext,
        stimulus: &Stimulus,
        ctx: &AiContext,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
        _doors: Option<&[crate::gate::Door]>,
    ) -> bool {
        let stimulus_type = stimulus.stimulus_type;

        match stimulus_type {
            StimulusType::EventView => {
                if let StimulusInfo::Human(human_handle) = stimulus.info {
                    match self.base.current_state {
                        AiState::Default | AiState::Wondering => {
                            self.event_view_standard_procedure(human_handle.get(), ctx);
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
                            if let Some(view) = ctx.entity_view(human_handle.get())
                                && ctx.is_hostile_with(view.camp)
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
                            let Some(v) = ctx.entity_view(human_handle.get()) else {
                                return false;
                            };
                            let different_camp = ctx.is_hostile_with(v.camp);
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
                            self.event_sees_body_standard_procedure(body_handle.get(), ctx);
                        }
                        _ => {
                            // Other states: ignore bodies
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

        let same_camp = ctx.is_allied_with(antagonist.camp);

        if same_camp {
            match self.base.current_state {
                AiState::Default | AiState::Wondering => {
                    // Wow! A hero!  Only emit the admire reaction
                    // for Robin specifically (PC identity plus Robin identity).
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
                self.panic_undirected(AI_STANDARD_PANIC_RUNS as u8);
            } else {
                // Outside — reaction time before alerting.
                self.base.primary_target = Some(AiEntityHandle::new(good_guy));
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

        self.random_speech_for_owner(
            sim,
            ctx.self_is_beggar,
            ctx.entity_view(self.base.me)
                .map(|view| view.current_animation),
        );
    }

    /// Ambient speech only observes its owner; production need not snapshot
    /// every other entity to supply these two inputs.
    pub(crate) fn random_speech_for_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        is_beggar: bool,
        animation: Option<crate::order::OrderType>,
    ) {
        // ---- executed only every 256 frames ----

        if is_beggar {
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
        // An owner excluded from the spatial observation has no animation
        // input, matching the full-context entity_view lookup.
        if animation == Some(crate::order::OrderType::Weeping) {
            self.base.say(Remark::CivCries);
        }
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
mod tests;
