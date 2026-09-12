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

/// Caller tail to run only when both synchronous soldier-alert route
/// attempts fail.
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum AlertSoldierFailureContinuation {
    PanicWithRemark,
    Panic,
    ReturnToDuty,
}

/// Geometry for NPC 360-degree detection as used when alerting a soldier.
///
/// Both original endpoints start from the actors' literal world positions
/// values. In particular, AI actor position may
/// already report a committed gate-side point while the actor's sprite is
/// still interpolating through a door, so the AI planning position is not a
/// valid substitute here.
fn alert_soldier_360_geometry(
    ctx: &AiContext,
    target: &crate::ai_entity_view::AiEntityView,
) -> (
    crate::coordinates::WorldPoint3D,
    crate::coordinates::WorldPoint3D,
    f32,
) {
    let mut viewer_eye = ctx.self_body_position_world;
    viewer_eye.z +=
        crate::stealth::eye_z_for_posture(crate::element::Posture::Upright, ctx.self_is_rider);
    let target_detection = crate::stealth::detection_point_world(
        target.detection_position_world,
        target.posture,
        target.direction as i16,
        target.is_rider,
    );
    let dx = target_detection.x - viewer_eye.x;
    let dy = (target_detection.y - viewer_eye.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
    let dz = target_detection.z - viewer_eye.z;
    (viewer_eye, target_detection, dx * dx + dy * dy + dz * dz)
}

// ---------------------------------------------------------------------------
// Civilian-specific constants
// ---------------------------------------------------------------------------

pub const APPLE_CHASE_IDEAL_DISTANCE: i32 = 300;
pub const BEGGAR_NO_RANDOM_TALK_DISTANCE: i32 = 100;

/// Truthful engine snapshot for the only cross-entity per-tick value consumed
/// by Friendly AI. Deliberately has no `Default`/`stub`: handlers that require
/// a patrol chief must demand the live snapshot contextually.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub(crate) struct FriendlyPerTickData {
    patrol_chief: Option<FriendlyPatrolChief>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
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
#[derive(Debug, Clone, robin_state_hash_derive::StateHash, bitcode::Encode, bitcode::Decode)]
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

    /// Apply common patrol geometry through friendly state changes.
    /// The base routine owns stop-all and formation planning; the friendly
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
    // post-decision time. The original-game flow is synchronous:
    // Select the nearest door, enter the fleeing-to-door state, then move to it.
    // The pure AI stages `FleeingPanic` so its remaining borrowed tail sees
    // the conservative fallback state. The engine-side request drain folds
    // that placeholder into the final door/no-door transition before script
    // callbacks run: Original performs the lookup synchronously and exposes
    // only that final state change to AI-event filtering.

    /// Panic fleeing from a specific point, tagged with the sector
    /// and level of its origin so the engine's door lookup can
    /// resolve multi-level flee paths correctly.
    fn panic_from_point_at(&mut self, center: Position, runs: u8) {
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
        self.base.lasting_panic_runs = runs;
        self.base.directed_panic = true;
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

    /// Undirected panic.
    fn panic_undirected(&mut self, runs: u8) {
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
            self.panic_undirected(AI_STANDARD_PANIC_RUNS as u8);
            return;
        }

        if matches!(continuation, AlertContinuation::CivilianReachedSoldier) {
            self.base
                .outbox
                .actor
                .delete_detectable_type(crate::element::DetectableType::Friend);
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
                .push(StimulusType::EventReachPoint.into()),
            AlertContinuation::CivilianSawSoldier => {
                self.base.say(Remark::CivCallsSoldier);
                let target = self
                    .base
                    .antagonist
                    .expect("accepted civilian alert requires a target soldier")
                    .get();
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

    /// Decision-tick admission work which precedes the script `FilterAIEvent` call.
    pub(crate) fn start_think_pre_filter(&mut self, stimulus: &Stimulus) {
        // Civilian pre-think pipeline.  Civilians normally never
        // hit `EventWasp` / `EventNet`, but the gates live on the
        // shared behavior so any scripted substate change could reach them;
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

    /// Decision-tick admission work after `FilterAIEvent`. SetAIState observes these
    /// gates but deliberately ignores the returned admission decision.
    pub(crate) fn start_think_post_filter(
        &mut self,
        stimulus: &Stimulus,
        ctx: &AiContext,
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

    pub(crate) fn end_think(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        _global: &mut AiGlobalState,
        ctx: &AiContext,
    ) {
        // The original game's end-think phase dispatches this event here and runs the
        // script FilterAIEvent gate before dispatch. Queue these as
        // same-frame self-stimuli so the engine-side drain can apply
        // that filter without re-entering the script VM through this
        // borrowed AI object. The three-tier depth gate still matches
        // original game: <100 queues the follow-up, 100..=110 bails to
        // returning to duty, 111+ drops it silently.

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
                self.base.outbox.reentrant.self_stimuli.push(event.into());
                // Original dispatches this event recursively before the
                // decrement, so the frame stays open until the cascade's
                // innermost Think unwinds (see `open_end_think_frames`).
                self.base.open_end_think_frames = self.base.open_end_think_frames.saturating_add(1);
                return;
            }
        } else {
            // The deep-recursion fallback returns to duty instead of running a
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
        // No continuation was queued: the innermost Think of a completion
        // cascade unwinds the whole chain of still-open ancestor frames —
        // the deferred equivalent of the stacked tick-completion decrements the
        // Original performs while returning out of the nested calls.
        let open = std::mem::take(&mut self.base.open_end_think_frames);
        self.base.think_recursion_depth = self
            .base
            .think_recursion_depth
            .saturating_sub(1)
            .saturating_sub(open);
    }

    // -----------------------------------------------------------------------
    // Expected-event civilian dispatcher
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
                    // we abandon patrol and return to duty. The
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
                    if !self.alert_soldier(
                        sim,
                        seek_pos,
                        0,
                        AlertSoldierFailureContinuation::PanicWithRemark,
                        ctx,
                        grid,
                        doors,
                    ) {
                        self.base.say(Remark::CivPanic);
                        let pos = self.base.seek_position;
                        self.panic_from_point_at(pos, AI_STANDARD_PANIC_RUNS as u8);
                    }
                }
            }

            Substate::WonderingCivilianBodyReactiontime => {
                if stimulus_type == StimulusType::EventTimer {
                    let seek_pos = self.base.seek_position;
                    if !self.alert_soldier(
                        sim,
                        seek_pos,
                        0,
                        AlertSoldierFailureContinuation::PanicWithRemark,
                        ctx,
                        grid,
                        doors,
                    ) {
                        self.base.say(Remark::CivPanic);
                        let pos = self.base.seek_position;
                        self.panic_from_point_at(pos, AI_STANDARD_PANIC_RUNS as u8);
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
                    let antagonist_handle = self
                        .base
                        .antagonist
                        .expect("running-to-soldier state requires an antagonist")
                        .get();
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
                            // on failure, fall back to returning to duty.
                            let seek_pos = self.base.seek_position;
                            if !self.alert_soldier(
                                sim,
                                seek_pos,
                                0,
                                AlertSoldierFailureContinuation::ReturnToDuty,
                                ctx,
                                grid,
                                doors,
                            ) {
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
                            target: self
                                .base
                                .antagonist
                                .expect("civilian report requires its target soldier")
                                .get(),
                            stimulus_type: StimulusType::CallReport,
                            info: StimulusInfo::Hint(Hint {
                                seek_point: self.base.seek_position,
                                seek_flags: 0,
                                who_tells_me: AiEntityHandle::new(self.base.me),
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
                    self.panic_from_point_at(pos, AI_STANDARD_PANIC_RUNS as u8);
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
    // Unexpected-event civilian dispatcher
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
                    self.base.antagonist = Some(soldier_handle);
                    // The original deletes friend detectables before the
                    // direct CALL_ALERT, including when the soldier refuses.
                    self.base
                        .outbox
                        .actor
                        .delete_detectable_type(crate::element::DetectableType::Friend);
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::RequestAlert {
                            target: soldier_handle.get(),
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
                        // human reference in the original game. Rust carries that
                        // target-specific view separately on `AiContext`, so
                        // the outer EVENT_AFTER_SCRIPT_GO_ON context cannot be
                        // reused unchanged for a retained EVENT_VIEW.
                        let mut nested_ctx = ctx.clone();
                        if let StimulusInfo::Human(handle) = q.info {
                            let view = nested_ctx.entity_view(handle.get()).unwrap_or_else(|| {
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
                        .think_expected_event_common_stuff(sim, stimulus, ctx);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx);
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
    // Alerting-event civilian dispatcher
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
                let origin = noise
                    .origin
                    .position()
                    .expect("delivered whistle noise has no spatial layer");
                self.base.seek_position = origin;
                self.base
                    .face_position_at_elevation_with_ctx(origin, noise.elevation as f32, ctx);
                self.base.launch_timer(70, ctx.frame);
            }
            NoiseType::Aaargh => {
                // Scream — try to alert a soldier
                let origin = noise
                    .origin
                    .position()
                    .expect("delivered scream noise has no spatial layer");
                self.base.seek_position = origin;

                // On a Royalist civilian's scream, the civilian
                // panics directly instead of alerting a (nearby,
                // also Royalist) soldier.
                let is_royalist = ctx.is_player_aligned();

                if is_royalist
                    || !self.alert_soldier(
                        sim,
                        origin,
                        0,
                        AlertSoldierFailureContinuation::Panic,
                        ctx,
                        grid,
                        doors,
                    )
                {
                    let pos = self.base.seek_position;
                    self.panic_from_point_at(pos, AI_STANDARD_PANIC_RUNS as u8);
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
    /// 2. Of the STATE_DEFAULT candidates, pick the maximum-norm-nearest
    ///    with a +1000 layer-change penalty for soldiers on a
    ///    different floor.
    /// 3. When `ALERTFLAG_CHECK_DOOR_PATH` is set *and* we have a
    ///    grid reference, reject candidates whose gate-graph path
    ///    from our sector is unroutable (lifts / locked doors).
    /// 4. Run to the picked soldier and transition to
    ///    SEEKING_CIVILIAN_RUNNING_TO_SOLDIER.
    pub const ALERTFLAG_CHECK_DOOR_PATH: u16 = 0x0001;

    /// clearing all friend detectables as issued from
    /// while alerting a soldier
    /// in each alert branch.
    ///
    /// Every one of those deletes runs *after* the loop's inline
    /// adding the soldier as a friend detectable
    /// before route validation, so a failed alert always leaves the
    /// FRIEND bucket empty. The actor outbox preserves that append/delete order,
    /// including entries not yet applied at the existing drain boundary.
    fn delete_all_friend_detectables(&mut self) {
        self.base
            .outbox
            .actor
            .delete_detectable_type(crate::element::DetectableType::Friend);
    }

    pub(crate) fn alert_soldier(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        center: Position,
        flags: u16,
        failure: AlertSoldierFailureContinuation,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        doors: Option<&[crate::gate::Door]>,
    ) -> bool {
        let my_pos = ctx.position;
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

        // Soldier lookup walks the camp's soldier registry, and
        // Detectable addition preserves that order. Hash-map iteration here made
        // the FRIEND list nondeterministic even when its membership matched.
        for &handle in ctx.all_soldier_handles.iter() {
            let view = ctx.entity_view(handle).unwrap_or_else(|| {
                panic!("alert-soldier registry handle {handle} has no live AI entity view")
            });
            if handle == self.base.me {
                continue;
            }
            if !view.is_soldier() || !ctx.is_allied_with(view.camp) {
                continue;
            }
            if !view.is_able_to_fight {
                continue;
            }
            // Skip script-locked soldiers entirely so a
            // scripted informative-wait guard isn't dragged off-
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
                    // AI maximum-norm distance subtracts
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
                    // candidate loses the maximum-norm comparison. Needs
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
                    let (viewer_eye, target_detection, square_distance) =
                        alert_soldier_360_geometry(ctx, view);
                    if square_distance <= sq_view_radius
                        && crate::sight_obstacle::is_reachable_3d(
                            ctx.obstacle_list(),
                            [viewer_eye.x, viewer_eye.y, viewer_eye.z],
                            [target_detection.x, target_detection.y, target_detection.z],
                            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
                        )
                    {
                        // Clear the friend list and return false.
                        // We queue the clear — the engine drains it
                        // post-think.
                        self.delete_all_friend_detectables();
                        return false;
                    }
                }
                _ => {}
            }
        }

        // Queue the friend-detectable adds we accumulated above. Original
        // adds detectables directly here: its uniqueness check is an
        // assert, so the retail build appends even when the friend is already
        // present. Keep these calls on the duplicate-preserving lane.
        // Done here (not inline) so the early-return above doesn't
        // add detectables we're about to drop.
        self.base.outbox.actor.detectable_mutations.extend(
            detectables_to_append
                .into_iter()
                .map(|(target, kind)| crate::ai::DetectableMutation::Append(target, kind)),
        );

        let Some((target_handle, _, target_pos)) = best else {
            // No candidate found — clear friend list and give up.
            self.delete_all_friend_detectables();
            return false;
        };

        self.base.antagonist = Some(AiEntityHandle::new(target_handle));
        self.base.seek_position = center;
        self.set_state(AiState::Seeking, Substate::SeekingCivilianRunningToSoldier);
        // Run toward the picked soldier's forecasted destination
        // (e.g. the far side of an in-flight door pass) rather
        // than the animated mid-traversal position.  `target_pos`
        // is resolved from `view.forecasted_destination` at this exact
        // original-game decision point, so a building-exit choice owns its RNG draw.
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
                    failure,
                    ctx,
                    grid,
                    doors,
                );
            }
            self.delete_all_friend_detectables();
            return false;
        }

        // Original-game path construction is synchronous. Close the approach actor
        // prefix, then let the engine-owned continuation inspect its result,
        // retry with CHECK_DOOR_PATH on failure, and only then execute the
        // caller's failure tail or the successful remark.
        self.base
            .outbox
            .reentrant
            .owner_work
            .push(crate::ai::AiOwnerWork::ActorEffects(std::mem::take(
                &mut self.base.outbox.actor,
            )));
        self.base.outbox.reentrant.alert_soldier_completion_pending = true;
        self.base.outbox.reentrant.owner_work.push(
            crate::ai::AiOwnerWork::ResumeFriendlyAlertSoldierAfterGoNear {
                center,
                check_door_path,
                failure,
            },
        );
        true
    }

    pub(crate) fn resume_alert_soldier_after_go_near(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        center: Position,
        check_door_path: bool,
        failure: AlertSoldierFailureContinuation,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        doors: Option<&[crate::gate::Door]>,
    ) {
        self.base.outbox.reentrant.alert_soldier_completion_pending = false;
        if !self.base.couldnt_reachpoint {
            self.base.say(Remark::CivPanic);
            return;
        }
        self.base.couldnt_reachpoint = false;
        if !check_door_path
            && self.alert_soldier(
                sim,
                center,
                Self::ALERTFLAG_CHECK_DOOR_PATH,
                failure,
                ctx,
                grid,
                doors,
            )
        {
            return;
        }
        if check_door_path {
            self.delete_all_friend_detectables();
        }
        match failure {
            AlertSoldierFailureContinuation::PanicWithRemark => {
                self.base.say(Remark::CivPanic);
                self.panic_from_point_at(center, AI_STANDARD_PANIC_RUNS as u8);
            }
            AlertSoldierFailureContinuation::Panic => {
                self.panic_from_point_at(center, AI_STANDARD_PANIC_RUNS as u8);
            }
            AlertSoldierFailureContinuation::ReturnToDuty => {
                self.return_to_duty(sim, DutyFlags::empty(), ctx);
            }
        }
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
                    // ladder mount), and re-issuing movement now would
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
                            // The original game retries the movement request, including
                            // its synchronous already-on-point callback.
                            self.base.go_to(dest, flags, ctx);
                        } else {
                            self.base
                                .outbox
                                .reentrant
                                .self_stimuli
                                .push(StimulusType::EventCouldntReachPoint.into());
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
        // and takes the else-branch's timer launch and
        // default / on-post state-change cascade below. (Re-reading
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
            // Movement requests check the AI decision-method recursion depth and
            // either sets `already_on_point` (for the enclosing
            // tick completion to dispatch) or fires a reach-point decision tick
            // directly when called outside a Think cycle.
            // `return_to_duty` runs outside AI decisions, so movement to a
            // waypoint we already stand on sets `already_on_point =
            // true` but nothing drains it — queue a self-stimulus
            // so the engine's next-tick drain dispatches it (same
            // shape as the enemy branch).
            if self.base.already_on_point {
                self.base.already_on_point = false;
                self.base
                    .fire_self_stimulus(crate::ai::StimulusType::EventReachPoint);
            }
            // A failed movement and a no-op facing command raise their latches
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

        // The original game stamps this after all patrol-path setup.
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
mod tests;
