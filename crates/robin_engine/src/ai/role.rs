//! Behaviour shared by the two typed AI roles (`EnemyAi`, `FriendlyAi`).
//!
//! Both roles wrap an [`AiController`] and override state selection and alert
//! status. The movement wrappers, patrol coordination and decision-tick
//! pre-filter admission are otherwise identical, so they live here once as
//! default methods; each role supplies only the hooks.

use super::{
    AiContext, AiController, AiState, AlertLevel, GotoFlags, PatrolCoordinateAction, Position,
    Stimulus, StimulusInfo, StimulusType, Substate,
};

pub(crate) trait AiRole {
    fn base_mut(&mut self) -> &mut AiController;

    /// The role's own `set_state` (alert status, script notification, ...).
    fn role_set_state(&mut self, state: AiState, substate: Substate);

    /// The role's own `set_alert_status`.
    fn role_set_alert_status(&mut self, level: AlertLevel);

    /// Runs after a patrol-coordinate walk/run order has been queued.
    /// `first_new_order` is the actor order-queue length before the order.
    fn after_patrol_move(&mut self, _first_new_order: usize) {}

    // -----------------------------------------------------------------------
    // Movement helpers — bundle set_state + go_to/go_near/go_to_speed
    //
    // Enforces "Shape 1" contract: every movement order issued by the AI
    // must specify the substate the AI is transitioning to.  Rationale:
    // `engine/movement.rs::process_pending_ai_orders` halts the actor
    // before dispatching the new move (`halt()` inside `go_to()`), and
    // the halt-teardown suppresses the EVENT_DONE that would normally
    // reach the AI.  Under the original contract this is safe because
    // the caller of `go_to()` also does a `set_state()` right before
    // — the AI is already in the new substate when the torn-down
    // sequence's EventDone would have arrived, so suppressing it is
    // correct.  In our port the halt fires in a separate tick,
    // decoupled from the AI's
    // set_state, so a caller that forgot to transition would leave the AI
    // wedged in a "waiting" substate (Parade/Reactiontime/etc.) with no
    // way out.  These wrappers remove the split: the substate commit is
    // in the same call as the movement intent; there's no way to queue a
    // move without naming the new substate.
    // -----------------------------------------------------------------------

    /// Transition to `(state, substate)` and queue a movement to `destination`.
    /// See the section comment above for why state+substate are required.
    #[track_caller]
    fn go_to(
        &mut self,
        state: AiState,
        substate: Substate,
        destination: Position,
        flags: GotoFlags,
        ctx: &AiContext,
    ) {
        self.role_set_state(state, substate);
        self.base_mut().go_to(destination, flags, ctx);
    }

    /// Like [`AiRole::go_to`] but with a speed modifier.
    #[track_caller]
    fn go_to_speed(
        &mut self,
        state: AiState,
        substate: Substate,
        destination: Position,
        flags: GotoFlags,
        speed: f32,
        ctx: &AiContext,
    ) {
        self.role_set_state(state, substate);
        self.base_mut().go_to_speed(destination, flags, speed, ctx);
    }

    /// Transition to `(state, substate)` and queue a "go near" movement
    /// (stops within `distance` of the destination).
    #[track_caller]
    fn go_near(
        &mut self,
        state: AiState,
        substate: Substate,
        destination: Position,
        distance: i32,
        flags: GotoFlags,
        ctx: &AiContext,
    ) {
        self.role_set_state(state, substate);
        self.base_mut().go_near(destination, distance, flags, ctx);
    }

    /// Apply common patrol geometry through the role's state changes.
    /// The base routine owns stop-all and formation planning; the role owns
    /// the state effects before the movement order and, via
    /// [`AiRole::after_patrol_move`], any post-order adjustment.
    fn coordinate_patrol(
        &mut self,
        info: &StimulusInfo,
        ctx: &AiContext,
        patrol_chief_position: Position,
    ) {
        let Some(action) =
            self.base_mut()
                .prepare_patrol_coordinate(info, ctx, patrol_chief_position)
        else {
            return;
        };

        match action {
            PatrolCoordinateAction::FaceChief { target } => {
                self.base_mut().face_position_with_ctx(target, ctx);
            }
            PatrolCoordinateAction::Walk {
                target,
                speed_factor,
            } => {
                let first_new_order = self.base_mut().outbox.actor.orders.len();
                let flags = GotoFlags::NO_HALT
                    | GotoFlags::DONT_STOP
                    | self.base_mut().default_path_walking_flags;
                self.go_to_speed(
                    AiState::Default,
                    Substate::DefaultPatrolEnroute,
                    target,
                    flags,
                    speed_factor,
                    ctx,
                );
                self.after_patrol_move(first_new_order);
            }
            PatrolCoordinateAction::Run { target } => {
                let first_new_order = self.base_mut().outbox.actor.orders.len();
                self.go_to(
                    AiState::Default,
                    Substate::DefaultPatrolEnrouteRunning,
                    target,
                    GotoFlags::RUN | GotoFlags::NO_HALT | GotoFlags::DONT_STOP,
                    ctx,
                );
                self.after_patrol_move(first_new_order);
            }
        }
    }

    /// Decision-tick admission work which precedes the script `FilterAIEvent` call.
    /// Kept separate so script-native SetAIState can yield through the VM at
    /// the exact callback boundary without aliasing the typed brain.
    fn start_think_pre_filter(&mut self, stimulus: &Stimulus) {
        let stimulus_type = stimulus.stimulus_type;
        let base = self.base_mut();

        // Reset per-think flags
        base.couldnt_reachpoint = false;
        base.already_on_point = false;
        base.already_turned = false;
        base.old_state = base.current_state as i32;
        // The roles used to disagree here (friendly `saturating_add`, enemy
        // `+= 1`). Depth 255 is not reachable: completion cascades stop
        // queueing nested Thinks at depth 100 (`end_think`), a resumed
        // continuation pins the depth to 100, and every other re-entry
        // (direct `think` re-dispatch, SetAIState decision entry, deferred
        // engine completion) adds a single bounded frame. Both spellings are
        // therefore observably identical; fail loudly on the impossible state
        // instead of silently saturating or wrapping.
        base.think_recursion_depth = base
            .think_recursion_depth
            .checked_add(1)
            .expect("think recursion depth overflow");

        // Track stimulus actor
        if let StimulusInfo::Human(h) = stimulus.info {
            base.last_stimulus_actor = Some(h);
        }

        // LOSE_CONSCIOUSNESS always drops the alert to green regardless of
        // the downstream refusal — even when the event is otherwise filtered
        // out.
        if stimulus_type == StimulusType::EventLoseConsciousness {
            self.role_set_alert_status(AlertLevel::Green);
        }
    }
}
