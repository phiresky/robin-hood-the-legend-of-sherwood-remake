//! Script VM/host-adapter wiring, mission script management, and campaign integration.

use super::scroll_reveal::ScrollStatus;
use super::*;
use crate::campaign::{Campaign, CampaignValue};
use crate::messenger::{Message, MessageType, SimpleMessage};
use crate::profiles::{MissionLocation, MissionProfile};

#[cfg(test)]
std::thread_local! {
    static ACTIVE_DRIVER_SNAPSHOT_PROBE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static ACTIVE_DRIVER_SNAPSHOT_ERROR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static AI_STATE_CALLBACK_OBSERVATIONS: std::cell::RefCell<Option<Vec<AiStateCallbackObservation>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AiStateCallbackObservation {
    pub owner: crate::element::EntityId,
    pub eye_status: crate::element::EyeStatus,
    pub timer_is_running: bool,
    pub body_references_to_owner: usize,
}

#[cfg(test)]
pub(super) fn capture_ai_state_callback_observations<R>(
    f: impl FnOnce() -> R,
) -> (R, Vec<AiStateCallbackObservation>) {
    AI_STATE_CALLBACK_OBSERVATIONS.with(|observations| {
        assert!(
            observations.borrow().is_none(),
            "AI state callback observation capture is already active"
        );
        *observations.borrow_mut() = Some(Vec::new());
    });
    let result = f();
    let observations = AI_STATE_CALLBACK_OBSERVATIONS.with(|observations| {
        observations
            .borrow_mut()
            .take()
            .expect("AI state callback observation capture disappeared")
    });
    (result, observations)
}

#[cfg(test)]
pub(super) fn arm_active_driver_snapshot_probe() {
    ACTIVE_DRIVER_SNAPSHOT_PROBE.with(|probe| probe.set(true));
    ACTIVE_DRIVER_SNAPSHOT_ERROR.with(|error| *error.borrow_mut() = None);
}

#[cfg(test)]
pub(super) fn take_active_driver_snapshot_error() -> Option<String> {
    ACTIVE_DRIVER_SNAPSHOT_ERROR.with(|error| error.borrow_mut().take())
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ActiveScriptCall {
    pub(super) target: ScriptVmKey,
    pub(super) frame: crate::natives::ScriptCallFrame,
    /// External-native entry supplies receiver context but is not a VM
    /// activation and therefore must not consume one recursion slot.
    pub(super) counts_toward_depth: bool,
}

#[derive(Debug)]
pub(super) struct ScriptDriverError {
    pub(super) detail: String,
    /// True once the sequence element that actually failed has been marked
    /// Impossible. Ancestor actions must not be blamed for descendant errors.
    pub(super) sequence_element_failed: bool,
}

impl ScriptDriverError {
    pub(super) fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            sequence_element_failed: false,
        }
    }
}

impl From<String> for ScriptDriverError {
    fn from(detail: String) -> Self {
        Self::new(detail)
    }
}

impl From<&str> for ScriptDriverError {
    fn from(detail: &str) -> Self {
        Self::new(detail)
    }
}

/// Script-originated effects removed from the VM adapter before processing.
///
/// Draining first ends the `MissionScript` borrow. Effect handlers may then
/// synchronously re-enter script dispatch while the canonical VM remains in
/// `ScriptRuntime`; no engine state or script owner is parked elsewhere.
impl EngineInner {
    /// Run one callback to completion, servicing every VM yield before the
    /// caller regains control. This is the sole engine-owned yield/resume
    /// boundary for all persistent script-instance kinds.
    pub(super) fn call_script_vm(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        key: ScriptVmKey,
        fn_name: &str,
        params: &[i32],
        frame: crate::natives::ScriptCallFrame,
    ) -> Result<i32, String> {
        self.call_script_vm_inner(sim, assets, key, fn_name, params, frame, &mut Vec::new())
            .map_err(|error| error.detail)
    }

    pub(super) fn call_script_vm_inner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        key: ScriptVmKey,
        fn_name: &str,
        params: &[i32],
        frame: crate::natives::ScriptCallFrame,
        active: &mut Vec<ActiveScriptCall>,
    ) -> Result<i32, ScriptDriverError> {
        let script =
            self.scripts.mission.as_ref().ok_or_else(|| {
                format!("cannot call {key:?}.{fn_name}: no mission script loaded")
            })?;
        if !script.has_script_vm(key) {
            return Err(format!("cannot call {key:?}.{fn_name}: required VM is not bound").into());
        }
        if !script.script_vm_has_function(key, fn_name) {
            return Ok(if fn_name == "FilterAIEvent" { 1 } else { 0 });
        }
        let real_depth = active
            .iter()
            .filter(|call| call.counts_toward_depth)
            .count();
        if real_depth >= usize::from(crate::natives::MAX_NESTED_CALL_DEPTH) {
            return Err(ScriptDriverError::new(format!(
                "nested script callback depth limit ({}) exceeded while calling {key:?}.{fn_name}",
                crate::natives::MAX_NESTED_CALL_DEPTH
            )));
        }

        let mut activation = self
            .with_script_session_in_driver(sim, assets, |script, _, _| {
                script.begin_script_vm(key, fn_name, params)
            })
            .expect("validated mission script vanished")?;

        self.scripts
            .mission
            .as_mut()
            .expect("validated mission script vanished before activation guard")
            .push_active_driver_frame(frame);
        active.push(ActiveScriptCall {
            target: key,
            frame,
            counts_toward_depth: true,
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.drive_started_script_vm(sim, assets, key, fn_name, frame, &mut activation, active)
        }));
        let popped = active.pop();
        debug_assert!(popped.is_some_and(|call| call.target == key));
        self.scripts
            .mission
            .as_mut()
            .expect("mission script vanished while restoring activation guard")
            .pop_active_driver_frame(frame);
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn drive_started_script_vm(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        key: ScriptVmKey,
        fn_name: &str,
        frame: crate::natives::ScriptCallFrame,
        activation: &mut crate::interp::VmActivationState,
        active: &mut Vec<ActiveScriptCall>,
    ) -> Result<i32, ScriptDriverError> {
        loop {
            let stop = self
                .with_script_session_in_driver(
                    sim,
                    assets,
                    |script, script_domains, capabilities| {
                        script.resume_script_vm(
                            key,
                            fn_name,
                            frame,
                            activation,
                            script_domains,
                            capabilities,
                        )
                    },
                )
                .expect("mission script vanished while callback was suspended");
            self.drain_script_effects_with_active(sim, assets, active)?;
            // `Thanx()` launches the recorded sequence directly through the
            // sequence manager rather than through `ScriptEffects`.  Its
            // initial WAIT/immediate elements execute from the launch call
            // stack in the Original, so close that synchronous registration
            // boundary before returning from (or resuming) the VM callback.
            //
            // Without this drain a zone callback that records
            // LockUser+SendMessage leaves both actions queued until the next
            // engine hourglass.  Actors then receive one extra movement tick
            // before the message's synchronous LockAI loop can stop them.
            self.drain_script_registration_inline_actions(sim, assets, active)?;
            match stop {
                crate::interp::StopReason::ReturnedValue(value) => return Ok(value),
                crate::interp::StopReason::Returned => return Ok(0),
                crate::interp::StopReason::Yield(request) => {
                    let operation_result = match request.operation {
                        crate::interp::NativeOperation::ScriptCall(call) => {
                            let nested_frame = match call.script_this {
                                crate::interp::NestedCallScriptThis::TargetActor => {
                                    frame.with_script_this(call.actor_handle)
                                }
                                crate::interp::NestedCallScriptThis::PreserveCaller => frame,
                            };
                            let nested_key = ScriptVmKey::Actor(call.actor_handle);
                            self.call_script_vm_inner(
                                sim,
                                assets,
                                nested_key,
                                &call.fn_name,
                                &call.params,
                                nested_frame,
                                active,
                            )?
                        }
                        crate::interp::NativeOperation::SequenceAction(operation) => {
                            self.drive_detached_sequence_operation(sim, assets, operation, active)?;
                            0
                        }
                        crate::interp::NativeOperation::EngineAction(action) => {
                            self.execute_synchronous_script_request(sim, assets, action, active)?
                        }
                    };
                    activation.native_return_value = match request.resume {
                        crate::interp::ResumePolicy::OperationResult => operation_result,
                        crate::interp::ResumePolicy::Fixed(value) => value,
                    };
                }
                crate::interp::StopReason::StepLimit => {
                    return Err(ScriptDriverError::new(format!(
                        "{key:?} script {fn_name} exceeded the VM step limit"
                    )));
                }
                other => {
                    return Err(ScriptDriverError::new(format!(
                        "{key:?} script {fn_name} stopped abnormally: {other:?}"
                    )));
                }
            }
        }
    }

    fn dispatch_script_ai_native_moves(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: crate::element::EntityId,
        active: &mut Vec<ActiveScriptCall>,
    ) -> Result<(), ScriptDriverError> {
        let launched_moves = self.drain_pending_move_requests_for_owner(sim, owner);
        for sequence_id in launched_moves {
            let action = self
                .orders
                .sequence_manager
                .take_deferred_owner_action(owner, sequence_id, 0)
                .map_err(|detail| {
                    ScriptDriverError::new(format!(
                        "SetAIState owner {} Move dispatch failed: {detail}",
                        owner.index()
                    ))
                })?;
            if let Some(action) = action {
                self.dispatch_script_synchronous_action(sim, assets, action, active)?;
                self.drain_script_synchronous_actions(sim, assets, active)?;
            }
        }
        self.drain_direct_ai_owner_boundary_without_forecast(sim, owner, assets);
        Ok(())
    }

    fn execute_synchronous_script_request(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        request: crate::interp::SynchronousScriptRequest,
        active: &mut Vec<ActiveScriptCall>,
    ) -> Result<i32, ScriptDriverError> {
        match request {
            crate::interp::SynchronousScriptRequest::ApplyAiStateNative {
                actor, effect, ..
            } => {
                let owner = self.entity_id_for_actor_handle(actor).ok_or_else(|| {
                    format!(
                        "SetAIState owner handle {actor} became stale before its synchronous effect barrier"
                    )
                })?;
                let entity = self.get_entity(owner).ok_or_else(|| {
                    format!(
                        "SetAIState owner {} disappeared before its synchronous effect barrier",
                        owner.index()
                    )
                })?;
                if !entity.is_npc() || entity.ai_controller().is_none() {
                    return Err(format!(
                        "SetAIState owner {} lost its required NPC AI before its synchronous effect barrier",
                        owner.index()
                    )
                    .into());
                }
                // RHArtificialIntelligence::SetAIState wraps SEEKING and
                // FLEEING in StartThink(NO_EVENT) / EndThink. StartThink runs
                // FilterAIEvent(NULL, -2) before its later freeze/lock gates;
                // its bool is ignored by SetAIState. The outer effect is not
                // installed yet, so a recursive SetAIState from this callback
                // can stabilize without consuming its caller's work.
                if matches!(
                    effect,
                    crate::interp::ScriptAiStateNativeEffect::Seeking
                        | crate::interp::ScriptAiStateNativeEffect::Fleeing
                ) {
                    self.start_script_ai_native_think_pre_filter(owner);
                    let is_scripted = self
                        .get_entity(owner)
                        .and_then(Entity::actor_data)
                        .is_some_and(|actor| !actor.script_class.is_empty());
                    let should_call = is_scripted
                        && sim.config().script_enabled
                        && self.scripts.mission.as_ref().is_some_and(|script| {
                            script.actor_has_function(actor, "FilterAIEvent")
                        });
                    let filter_accepted = if should_call {
                        match self.call_script_vm_inner(
                            sim,
                            assets,
                            ScriptVmKey::Actor(actor),
                            "FilterAIEvent",
                            &[0, -2],
                            crate::natives::ScriptCallFrame::actor(actor),
                            active,
                        ) {
                            Ok(value) => value != 0,
                            Err(error) => {
                                tracing::warn!(
                                    actor_handle = actor,
                                    %error.detail,
                                    "SetAIState StartThink(NO_EVENT) FilterAIEvent callback failed — allowing"
                                );
                                true
                            }
                        }
                    } else {
                        true
                    };
                    if filter_accepted {
                        let _ = self.start_script_ai_native_think_post_filter(owner);
                    }
                }

                match effect {
                    crate::interp::ScriptAiStateNativeEffect::ScriptDriven => {
                        self.set_typed_npc_state(
                            owner,
                            crate::ai::AiState::Default,
                            crate::ai::Substate::DefaultScriptDriven,
                            "SetAIState SCRIPT_DRIVEN",
                        );
                    }
                    crate::interp::ScriptAiStateNativeEffect::Default
                    | crate::interp::ScriptAiStateNativeEffect::Seeking
                    | crate::interp::ScriptAiStateNativeEffect::Fleeing => {
                        let current_position = {
                            let entity = self.get_entity(owner).unwrap_or_else(|| {
                                panic!(
                                    "SetAIState owner {} disappeared after NO_EVENT callback",
                                    owner.index()
                                )
                            });
                            let data = entity.element_data();
                            let point = data.position_map();
                            crate::ai::Position {
                                x: point.x,
                                y: point.y,
                                sector: data.sector(),
                                level: data.layer(),
                            }
                        };
                        let state = match effect {
                            crate::interp::ScriptAiStateNativeEffect::Default => {
                                crate::ai::AiState::Default
                            }
                            crate::interp::ScriptAiStateNativeEffect::Seeking => {
                                crate::ai::AiState::Seeking
                            }
                            crate::interp::ScriptAiStateNativeEffect::Fleeing => {
                                crate::ai::AiState::Fleeing
                            }
                            crate::interp::ScriptAiStateNativeEffect::ScriptDriven => {
                                unreachable!()
                            }
                        };
                        self.get_entity_mut(owner)
                            .and_then(Entity::ai_controller_mut)
                            .unwrap_or_else(|| {
                                panic!(
                                    "SetAIState owner {} lost its required typed AI after NO_EVENT callback",
                                    owner.index()
                                )
                            })
                            .script_set_ai_state(state, current_position);
                    }
                }

                self.drain_direct_ai_owner_boundary_without_forecast(sim, owner, assets);
                if let Err(error) = self.dispatch_script_ai_native_moves(sim, assets, owner, active)
                {
                    return Err(error);
                }
                if matches!(
                    effect,
                    crate::interp::ScriptAiStateNativeEffect::Seeking
                        | crate::interp::ScriptAiStateNativeEffect::Fleeing
                ) {
                    self.end_script_ai_native_think(sim, assets, owner);
                    self.drain_direct_ai_owner_boundary_without_forecast(sim, owner, assets);
                    if let Err(error) =
                        self.dispatch_script_ai_native_moves(sim, assets, owner, active)
                    {
                        return Err(error);
                    }
                }
                Ok(0)
            }
            crate::interp::SynchronousScriptRequest::SetPersistentLifePoints {
                actor,
                amount,
                ..
            } => {
                let actor = self
                    .entity_id_for_actor_handle(actor)
                    .ok_or_else(|| format!("invalid actor handle {actor}"))?;
                self.apply_scripted_life_points(sim, assets, actor, amount);
                Ok(0)
            }
            crate::interp::SynchronousScriptRequest::SetPersistentConcussion {
                actor,
                amount,
                ..
            } => {
                let actor = self
                    .entity_id_for_actor_handle(actor)
                    .ok_or_else(|| format!("invalid actor handle {actor}"))?;
                self.get_entity(actor)
                    .expect("validated concussion actor vanished before synchronous apply")
                    .human_data()
                    .expect("validated concussion actor lost HumanData before synchronous apply");
                // C++ first narrows to UWORD, then tests the stored value as
                // SWORD. Values with bit 15 set become negative and normalize
                // to zero before the upper concussion cap is considered.
                let narrowed = amount as u16;
                let value = if (narrowed as i16) < 0 { 0 } else { narrowed };
                self.apply_scripted_concussion(sim, assets, actor, value, true);
                self.drain_pending_concussion_side_effects(sim, assets);
                Ok(0)
            }
            crate::interp::SynchronousScriptRequest::SetActorPosture { actor, posture, .. } => {
                self.apply_script_actor_posture(sim, assets, actor, posture, active)?;
                Ok(0)
            }
            crate::interp::SynchronousScriptRequest::SetActorLocation {
                actor, location, ..
            } => Ok(self.apply_script_actor_location(sim, assets, actor, location)?),
            crate::interp::SynchronousScriptRequest::SetActorActionState {
                actor, state, ..
            } => {
                let actor_id = self
                    .entity_id_for_actor_handle(actor)
                    .ok_or_else(|| format!("SetActorActionState invalid actor {actor}"))?;
                let state = crate::element::ActionState::try_from(state as u32)
                    .map_err(|_| format!("SetActorActionState invalid state {state}"))?;
                self.get_entity_mut(actor_id)
                    .expect("validated SetActorActionState actor vanished")
                    .actor_data_mut()
                    .expect("validated SetActorActionState human lost ActorData")
                    .action_state = state;
                self.actor_wait(actor_id);
                self.drain_script_synchronous_actions(sim, assets, active)?;
                Ok(0)
            }
            crate::interp::SynchronousScriptRequest::LockAi {
                actor,
                remember_events,
                ..
            } => {
                let owner = self.entity_id_for_actor_handle(actor).ok_or_else(|| {
                    format!("LockAI owner handle {actor} became stale at its synchronous barrier")
                })?;
                let from_lockai_command = self
                    .orders
                    .sequence_manager
                    .current_element_for_actor(owner)
                    .is_some_and(|(sequence_id, element_index)| {
                        self.orders
                            .sequence_manager
                            .get_element(sequence_id, element_index)
                            .is_some_and(|element| {
                                element.command == crate::element::Command::LockAi
                            })
                    });
                let ai = self
                    .get_entity_mut(owner)
                    .and_then(Entity::ai_controller_mut)
                    .ok_or_else(|| {
                        format!(
                            "LockAI owner {} lost its required NPC AI at its synchronous barrier",
                            owner.index()
                        )
                    })?;

                // Apply the lock and macro teardown now, but suppress the
                // controller's deferred Halt: Original immediately calls
                // actor.Stop(NORMAL), which must finish before the VM resumes
                // and launches any scripted replacement sequence.
                ai.script_lock(remember_events, true);
                if !from_lockai_command {
                    self.stop_owner(owner, crate::sequence::SequencePriority::Normal);
                    self.dispatch_condolations_in_script_driver(sim, assets, active)?;
                }
                Ok(0)
            }
            crate::interp::SynchronousScriptRequest::UnlockAi { actor, .. } => {
                let owner = self.entity_id_for_actor_handle(actor).ok_or_else(|| {
                    format!("UnlockAI owner handle {actor} became stale at its synchronous barrier")
                })?;
                let unconscious = self
                    .get_entity(owner)
                    .and_then(Entity::human_data)
                    .is_some_and(|human| human.unconscious);
                let ai = self
                    .get_entity_mut(owner)
                    .and_then(Entity::ai_controller_mut)
                    .ok_or_else(|| {
                        format!(
                            "UnlockAI owner {} lost its required NPC AI at its synchronous barrier",
                            owner.index()
                        )
                    })?;
                if ai.script_locked {
                    ai.script_unlock(unconscious);
                } else {
                    tracing::warn!(owner = owner.index(), "UnlockAI: NPC is not script-locked");
                }
                // ScriptUnlockAI synchronously re-enters
                // Think(EVENT_RETURN_TO_DUTY). Finish that owner-local call,
                // then materialize any resulting GoTo before resuming the VM.
                // Normal-priority movement remains registered for
                // SequenceManager::Hourglass; it is not instructed inline.
                self.drain_direct_ai_owner_boundary_without_forecast_deferred_instruct(
                    sim, owner, assets,
                );
                self.drain_pending_move_requests_for_owner(sim, owner);
                Ok(0)
            }
            crate::interp::SynchronousScriptRequest::AssignPath { actor, way, .. } => {
                let owner = self.entity_id_for_actor_handle(actor).ok_or_else(|| {
                    format!(
                        "AssignPath owner handle {actor} became stale at its synchronous barrier"
                    )
                })?;
                let entity = self.get_entity(owner).ok_or_else(|| {
                    format!(
                        "AssignPath owner {} disappeared at its synchronous barrier",
                        owner.index()
                    )
                })?;
                let data = entity.element_data();
                let current_position = crate::ai::Position {
                    x: data.position_map().x,
                    y: data.position_map().y,
                    sector: data.sector(),
                    level: data.layer(),
                };
                let current_direction = entity.position_iface().get_direction().as_u8() as u16;
                let assignment = if way == 0 {
                    crate::ai::PatrolAssignment::ClearPath
                } else if way == -1 {
                    crate::ai::PatrolAssignment::ClearPathSitAround
                } else {
                    crate::ai::PathId::new(way as u16)
                        .map(crate::ai::PatrolAssignment::Index)
                        .unwrap_or(crate::ai::PatrolAssignment::ClearPath)
                };
                let ai = self
                    .get_entity_mut(owner)
                    .and_then(Entity::ai_controller_mut)
                    .ok_or_else(|| {
                        format!(
                            "AssignPath owner {} lost its required NPC AI at its synchronous barrier",
                            owner.index()
                        )
                    })?;
                ai.assign_new_patrol_path(
                    assignment,
                    current_position,
                    current_direction,
                    &assets.hiking_paths,
                );
                // AssignNewPatrolPath synchronously runs
                // Think(EVENT_RETURN_TO_DUTY), including GoTo and its
                // LaunchSequenceElement call. Normal-priority Move does not
                // call Go/Instruct inline, though: it remains registered for
                // SequenceManager::Hourglass after the entity loop. Close the
                // AI callback now, then materialize its pending Move sequence
                // without instructing or executing it early.
                self.drain_direct_ai_owner_boundary_without_forecast(sim, owner, assets);
                self.drain_pending_move_requests_for_owner(sim, owner);
                Ok(0)
            }
            crate::interp::SynchronousScriptRequest::AssignPost {
                actor,
                post_x,
                post_y,
                direction,
                ..
            } => {
                let owner = self.entity_id_for_actor_handle(actor).ok_or_else(|| {
                    format!(
                        "AssignPost owner handle {actor} became stale at its synchronous barrier"
                    )
                })?;
                let entity = self.get_entity(owner).ok_or_else(|| {
                    format!(
                        "AssignPost owner {} disappeared at its synchronous barrier",
                        owner.index()
                    )
                })?;
                let data = entity.element_data();
                let post_position = crate::ai::Position {
                    x: post_x,
                    y: post_y,
                    sector: data.sector(),
                    level: data.layer(),
                };
                let ai = self
                    .get_entity_mut(owner)
                    .and_then(Entity::ai_controller_mut)
                    .ok_or_else(|| {
                        format!(
                            "AssignPost owner {} lost its required NPC AI at its synchronous barrier",
                            owner.index()
                        )
                    })?;
                ai.assign_new_post(post_position, direction as u16);
                // Original AssignNewPost calls Think(EVENT_RETURN_TO_DUTY)
                // before returning to the script. Close that owner-local AI
                // stack and materialize its GoTo now. The resulting ordinary
                // Move remains registered for the later sequence-manager
                // Hourglass, exactly like AssignPath above.
                self.drain_direct_ai_owner_boundary_without_forecast(sim, owner, assets);
                self.drain_pending_move_requests_for_owner(sim, owner);
                Ok(0)
            }
        }
    }

    fn apply_script_actor_location(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_handle: i32,
        location: i32,
    ) -> Result<i32, String> {
        let actor = self
            .entity_id_for_actor_handle(actor_handle)
            .ok_or_else(|| format!("SetActorLocation invalid actor handle {actor_handle}"))?;

        if location != 0 {
            if self
                .get_entity(actor)
                .expect("validated SetActorLocation actor vanished")
                .element_data()
                .in_honolulu
            {
                let entity = self
                    .get_entity_mut(actor)
                    .expect("validated SetActorLocation actor vanished before activation");
                entity.element_data_mut().active = true;
                entity.element_data_mut().in_honolulu = false;
            }

            let script = self
                .scripts
                .mission
                .as_ref()
                .expect("SetActorLocation requires the active mission script");
            let location_index = crate::natives::ScriptHandleCodec::location_index(location);
            let resolved = location_index.and_then(|index| {
                if index < script.bindings.script_location_count {
                    // Static sector handles follow points in the original
                    // script-object array and are deliberately rejected only
                    // after Honolulu reactivation.
                    if index >= script.bindings.script_point_count {
                        return None;
                    }
                    Some((
                        *script.bindings.location_positions.get(index)?,
                        Some((
                            *script.bindings.location_layers.get(index)?,
                            *script.bindings.location_sectors.get(index)?,
                        )),
                    ))
                } else {
                    let computed = script
                        .state
                        .computed_locations
                        .get(index - script.bindings.script_location_count)?
                        .as_ref()?;
                    Some((computed.position, computed.layer.zip(computed.sector)))
                }
            });
            let Some(((x, y), dest_layer_sector)) = resolved else {
                tracing::warn!(
                    "SetActorLocation: location {location} is invalid or is not a point"
                );
                return Ok(0);
            };

            // Original RHScript.cpp writes map position/layer/sector before
            // discovering that the referenced sector is not a motion area.
            if let Some((layer, sector)) = dest_layer_sector {
                let element = self
                    .get_entity_mut(actor)
                    .expect("validated SetActorLocation actor vanished before partial write")
                    .element_data_mut();
                element.set_position_map(crate::coordinates::MapPoint { x, y });
                element.set_layer(layer);
                element.set_sector(crate::position_interface::SectorHandle::new(sector));
                let valid_motion = self.world.fast_grid.level.sectors.iter().any(|candidate| {
                    candidate.sector_number.get() == sector as i16
                        && candidate.layer == layer
                        && candidate.sector_type.is_motion()
                        && candidate.sector_type.is_area()
                });
                if !valid_motion {
                    tracing::warn!(
                        "SetActorLocation: location {location} references non-motion sector {sector}"
                    );
                    return Ok(0);
                }
            }
            self.apply_host_commands(
                sim,
                assets,
                vec![crate::natives::EngineCommand::SetActorLocation {
                    actor_handle,
                    x,
                    y,
                    dest_layer_sector,
                    spawn_elevation_probe: None,
                }],
            );
            return Ok(1);
        }

        {
            let entity = self
                .get_entity_mut(actor)
                .expect("validated SetActorLocation actor vanished before Honolulu mutation");
            entity.element_data_mut().active = false;
            entity.element_data_mut().in_honolulu = true;
        }
        let is_human = self
            .get_entity(actor)
            .expect("SetActorLocation actor vanished after Honolulu mutation")
            .is_human();
        if is_human {
            self.quit_swordfight(sim, assets, actor);
            let still_unconscious = self
                .get_entity(actor)
                .expect("SetActorLocation human vanished after QuitSwordFight")
                .human_data()
                .expect("SetActorLocation validated human lost HumanData")
                .unconscious;
            self.feedback.titbit_manager.remove_unconscious_stars_if(
                crate::titbit::ElementHandle(actor.index()),
                still_unconscious,
            );
        }
        let mut disabled_pc = false;
        match self
            .get_entity_mut(actor)
            .expect("SetActorLocation actor vanished before playability/AI mutation")
        {
            crate::element::Entity::Pc(pc) if pc.pc.playable => {
                pc.pc.playable = false;
                disabled_pc = true;
                self.orders.messenger.send(crate::messenger::Message::pc(
                    crate::messenger::PcMessage::DisableCharacter,
                    Some(actor),
                ));
            }
            entity if entity.is_npc() => {
                let ai = entity
                    .ai_controller_mut()
                    .expect("SetActorLocation resolved NPC without an AI controller");
                if !ai.script_locked {
                    ai.script_lock(false, false);
                }
            }
            _ => {}
        }
        if disabled_pc {
            self.unselect_single_pc(actor);
        }
        Ok(1)
    }

    /// `RHScript::SetActorPosture` translated at the engine boundary so every
    /// Stop/SetPosture/Wait/concussion/death step completes in source order
    /// before the VM observes its next opcode.
    fn apply_script_actor_posture(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_handle: i32,
        posture_id: i32,
        active: &mut Vec<ActiveScriptCall>,
    ) -> Result<(), ScriptDriverError> {
        use crate::element::{ActionState, Posture};

        let actor = self
            .entity_id_for_actor_handle(actor_handle)
            .ok_or_else(|| format!("SetActorPosture invalid actor handle {actor_handle}"))?;
        let (current, is_npc) = self
            .get_entity(actor)
            .filter(|entity| entity.is_human())
            .map(|entity| (entity.element_data().posture, entity.is_npc()))
            .ok_or_else(|| format!("SetActorPosture target {actor_handle} is not human"))?;

        let set_posture = |engine: &mut Self, posture| {
            engine
                .get_entity_mut(actor)
                .expect("validated posture actor vanished")
                .set_posture(posture);
        };
        let wait = |engine: &mut Self, active: &mut Vec<ActiveScriptCall>| {
            engine.actor_wait(actor);
            engine.drain_script_synchronous_actions(sim, assets, active)
        };
        let clear_concussion = |engine: &mut Self| {
            engine.apply_scripted_concussion(sim, assets, actor, 0, true);
            engine.drain_pending_concussion_side_effects(sim, assets);
        };
        let notify_down = |engine: &mut Self| {
            if is_npc {
                engine.dispatch_ai_stimulus(
                    actor,
                    crate::ai::Stimulus::new(crate::ai::StimulusType::EventLoseConsciousness),
                );
                engine.broadcast_body_detectable(actor);
            }
        };

        match posture_id {
            0 => {
                if current == Posture::CarryingCorpse {
                    assert!(
                        self.get_entity(actor)
                            .expect("validated carrying actor vanished")
                            .is_pc(),
                        "SetActorPosture(UPRIGHT) from CarryingCorpse requires a PC"
                    );
                }
                if current == Posture::Lying && is_npc {
                    self.broadcast_resurrection(actor);
                }
                set_posture(self, Posture::Upright);
                wait(self, active)?;
                if current != Posture::CarryingCorpse {
                    clear_concussion(self);
                }
            }
            2 => {
                set_posture(self, Posture::Lying);
                wait(self, active)?;
                clear_concussion(self);
            }
            7 => {
                notify_down(self);
                set_posture(self, Posture::Tied);
                wait(self, active)?;
            }
            10 => {
                set_posture(self, Posture::Crouched);
                self.get_entity_mut(actor)
                    .expect("validated crouched actor vanished")
                    .actor_data_mut()
                    .expect("validated crouched human lost ActorData")
                    .action_state = ActionState::Waiting;
                wait(self, active)?;
            }
            15 => {
                self.apply_scripted_life_points(sim, assets, actor, 0);
                let entity = self
                    .get_entity_mut(actor)
                    .expect("validated dead actor vanished after virtual Kill");
                entity.set_posture(Posture::Dead);
                entity
                    .actor_data_mut()
                    .expect("validated dead human lost ActorData")
                    .action_state = ActionState::Waiting;
                wait(self, active)?;
            }
            16 => {
                set_posture(self, Posture::Sitting);
                clear_concussion(self);
                wait(self, active)?;
            }
            17 => {
                self.stop_owner(actor, crate::sequence::SequencePriority::Injury);
                self.drain_script_synchronous_actions(sim, assets, active)?;
                set_posture(self, Posture::Lying);
                self.apply_scripted_concussion(
                    sim,
                    assets,
                    actor,
                    crate::combat::CONCUSSION_MAX,
                    true,
                );
                self.drain_pending_concussion_side_effects(sim, assets);
                notify_down(self);
                wait(self, active)?;
            }
            100 => {
                set_posture(self, Posture::AnonymousArcher);
                self.get_entity_mut(actor)
                    .expect("validated AnonymousArcher actor vanished")
                    .actor_data_mut()
                    .expect("validated AnonymousArcher human lost ActorData")
                    .action_state = ActionState::Waiting;
                self.add_hidden_titbit_for_script_actor(assets, actor_handle, actor);
                wait(self, active)?;
            }
            forbidden @ (4 | 5 | 6 | 8 | 9 | 11) => {
                tracing::warn!(
                    "Script Error: SetActorPosture cannot set posture {forbidden} from script"
                );
            }
            other => tracing::warn!("Script Error: SetActorPosture illegal ID {other}"),
        }
        Ok(())
    }

    fn add_hidden_titbit_for_script_actor(
        &mut self,
        assets: &LevelAssets,
        actor_handle: i32,
        actor: crate::element::EntityId,
    ) {
        let entity = self
            .get_entity(actor)
            .expect("validated anonymous-archer actor vanished");
        assert!(
            entity.is_human(),
            "SetActorPosture(ANONYMOUS_ARCHER) requires a human actor {actor_handle}"
        );
        let phase = if let crate::element::Entity::Pc(pc) = entity {
            let profile = assets
                .profile_manager
                .get_character(pc.pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "anonymous archer PC {} has unknown profile_index {}",
                        actor.index(),
                        pc.pc.profile_index
                    )
                });
            crate::titbit::HiddenCharacter::for_pc(pc.pc.robin, &profile.filename).to_phase()
        } else {
            0
        };
        let handle = crate::titbit::ElementHandle(actor.index());
        self.feedback.titbit_manager.add_titbit(
            crate::coordinates::WorldPoint3D::default(),
            0,
            crate::titbit::TitbitKind::Hidden,
            handle,
            phase,
            handle,
            false,
            0,
            true,
            None,
            None,
        );
    }

    /// Canonical entry boundary for global, actor, zone, target, scroll, and
    /// waypoint script callbacks. The VM and every native capability are
    /// disjoint borrows of their sole owners; nothing is removed from
    /// `EngineInner`, including while nested callbacks resume the outer VM.
    pub(super) fn with_script_session<R>(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        callback: impl FnOnce(
            &mut MissionScript,
            &mut crate::engine::ScriptDomains,
            &crate::natives::NativeSessionCapabilities<'_>,
        ) -> R,
    ) -> Option<R> {
        if let Some(script) = self.scripts.mission.as_ref() {
            script.assert_no_active_call_frames();
        }
        let result = self.with_script_session_in_driver(sim, assets, callback);
        self.drain_script_effects_with_active(sim, assets, &mut Vec::new())
            .unwrap_or_else(|error| panic!("script effect drain failed: {}", error.detail));
        if let Some(script) = self.scripts.mission.as_ref() {
            script.assert_no_active_call_frames();
        }
        result
    }

    fn with_script_session_in_driver<R>(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        callback: impl FnOnce(
            &mut MissionScript,
            &mut crate::engine::ScriptDomains,
            &crate::natives::NativeSessionCapabilities<'_>,
        ) -> R,
    ) -> Option<R> {
        // Unlike `with_script_session`, this helper deliberately does not
        // assert an empty MissionScript call stack: real activation markers
        // remain installed across VM suspension, effect application, and all
        // nested drains. The owning driver pushes/pops them in catch-unwind
        // guards; external-native receiver frames follow the same lifetime but
        // are marked as depth-neutral in `ActiveScriptCall`.
        self.scripts.assert_native_attachments_ready();
        let result = {
            let EngineInner {
                mission_domain,
                control,
                ai,
                world,
                script_domains,
                orders,
                scripts,
                players,
                feedback,
            } = self;
            let script = scripts.mission.as_mut()?;
            let campaign = &mut mission_domain.campaign;
            let capabilities = crate::natives::NativeSessionCapabilities::new(
                sim,
                &mut world.entities,
                &mut ai.global,
                &mut world.fast_grid,
            )
            .with_world_views(
                assets.static_sight_obstacles.as_slice(),
                &world.dynamic_sight_obstacles,
                &world.static_sight_obstacle_active,
            )
            .with_queries(
                &mut orders.sequence_manager,
                &mut players.seats[0].selection,
                &mut feedback.sound_sim.sources,
                &world.weather,
                &control.frame_counter,
            )
            .with_campaign(campaign, &mut mission_domain.mission_stat)
            .with_short_briefings(&mut mission_domain.short_briefings)
            .with_standard_view_radius(&mut ai.standard_view_polygon_radius);
            let result = callback(script, script_domains, &capabilities);
            result
        };
        Some(result)
    }

    /// Attach immutable level data to the script-native dispatcher.
    ///
    /// The dispatcher borrows this object for each VM resume. It is not part
    /// of simulation state and is reattached after save/snapshot decode.
    pub(super) fn attach_script_bindings(&mut self, assets: &LevelAssets) {
        self.scripts.attach_native_capabilities(assets);
    }

    /// Drain and apply script-originated effects after a callback batch.
    ///
    /// The queue batch is removed under a short `ScriptRuntime` borrow before
    /// any effect is executed. Handlers can therefore re-enter the same live
    /// VM synchronously without a take/restore ownership transaction.
    fn drain_script_effects_with_active(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<ActiveScriptCall>,
    ) -> Result<(), ScriptDriverError> {
        loop {
            let (effect, parent_tail) = match self.scripts.mission.as_mut() {
                Some(script) => {
                    let Some(effect) = script.script_effects.pop_front() else {
                        return Ok(());
                    };
                    (effect, script.script_effects.take_tail())
                }
                None => return Ok(()),
            };
            #[cfg(test)]
            ACTIVE_DRIVER_SNAPSHOT_PROBE.with(|probe| {
                if probe.replace(false) {
                    let error = serde_json::to_string(&*self)
                        .expect_err("active driver must reject full EngineInner serialization")
                        .to_string();
                    ACTIVE_DRIVER_SNAPSHOT_ERROR.with(|slot| *slot.borrow_mut() = Some(error));
                }
            });
            let (sound, engine_commands, deferred) = match effect {
                crate::natives::ScriptEffect::ExternalSound(command) => {
                    (vec![command], Vec::new(), Vec::new())
                }
                crate::natives::ScriptEffect::Presentation(command)
                | crate::natives::ScriptEffect::Simulation(
                    crate::natives::SimulationEffect::Engine(command),
                ) => (Vec::new(), vec![command], Vec::new()),
                crate::natives::ScriptEffect::Simulation(
                    crate::natives::SimulationEffect::Deferred(command),
                ) => (Vec::new(), Vec::new(), vec![command]),
            };

            // Commands whose handlers can synchronously call the mission VM are
            // kept until the first-pass state effects have completed.
            let mut post_script: Vec<crate::natives::DeferredCommand> = Vec::new();

            // ── Sound commands ──
            // Commands that don't need an AudioBackend are processed now.
            // The remaining ones are queued for main_entry to flush.
            for cmd in sound {
                match cmd {
                    crate::natives::SoundCommand::SuspendAll => {
                        // SuspendAllSoundSources stops the audio
                        // channels but the paired `ResumeAll` must be
                        // able to restart every source that was active
                        // at suspend time.  We clear `active` so the
                        // hourglass stops channels, but first stash the
                        // active set on `sound_sim` so `ResumeAll` can
                        // restore it.
                        let mut stashed: Vec<u32> = Vec::new();
                        for i in 0..self.feedback.sound_sim.sources.num_sources() {
                            if let Some(src) = self.feedback.sound_sim.sources.get_mut(i)
                                && src.active
                            {
                                stashed.push(i as u32);
                                src.active = false;
                            }
                        }
                        self.feedback.sound_sim.suspended_active_sources = stashed;
                        self.feedback.sound_sim.playing_sources.clear();
                    }
                    crate::natives::SoundCommand::ResumeAll => {
                        // Restore `active` on every source that was
                        // active at the last suspend — preserves the
                        // active flag across suspend/resume.
                        let stashed =
                            std::mem::take(&mut self.feedback.sound_sim.suspended_active_sources);
                        for idx in stashed {
                            if let Some(src) = self.feedback.sound_sim.sources.get_mut(idx as usize)
                            {
                                src.active = true;
                            }
                        }
                        let pos = self.feedback.cutscene_camera.view_position;
                        let zoom = self.feedback.cutscene_camera.zoom_factor;
                        self.feedback.pending_side_effects.sounds.push(
                            super::SoundCommand::ResumeAllSources {
                                position: pos,
                                zoom,
                            },
                        );
                        // For every still-active `Single` / `Volatile`
                        // source that's being resumed, re-arm the
                        // deterministic finish so the drain in
                        // `perform_hourglass` applies the same
                        // transition the host used to drive from
                        // `stop_sound_source`.
                        schedule_source_finishes_for_all_active(
                            &mut self.feedback.sound_sim,
                            &assets.source_durations,
                            self.control.frame_counter,
                        );
                    }
                    crate::natives::SoundCommand::Activate(h) => {
                        // Mark active sim-side (participates in rollback hash),
                        // then emit the side-effect so the host audio backend
                        // picks up the source and starts a channel.  Symmetric
                        // with the Deactivate path below.
                        if let Some(idx) = crate::natives::ScriptHandleCodec::sound_source_index(h)
                        {
                            // Re-activation cancels any previously
                            // scheduled finish so we don't prematurely
                            // kill a freshly-restarted source.
                            self.feedback
                                .sound_sim
                                .playing_sources
                                .retain(|p| p.source_index as usize != idx);
                            if let Some(src) = self.feedback.sound_sim.sources.get_mut(idx) {
                                src.active = true;
                                schedule_source_finish(
                                    &src.source_kind,
                                    src.id,
                                    idx,
                                    self.control.frame_counter,
                                    &assets.source_durations,
                                    &mut self.feedback.sound_sim.playing_sources,
                                );
                            }
                            self.feedback
                                .pending_side_effects
                                .sounds
                                .push(super::SoundCommand::ActivateSource(idx));
                        }
                    }
                    crate::natives::SoundCommand::Deactivate(h) => {
                        // Mark inactive; hourglass will stop the channel.
                        // Drop any pending scheduled finish — the source
                        // is no longer playing and a stale `finish_frame`
                        // would fire as a no-op on an already-inactive
                        // source, but clearing it keeps the queue small
                        // and unambiguous across rollback snapshots.
                        if let Some(idx) = crate::natives::ScriptHandleCodec::sound_source_index(h)
                        {
                            if let Some(src) = self.feedback.sound_sim.sources.get_mut(idx) {
                                src.active = false;
                            }
                            self.feedback
                                .sound_sim
                                .playing_sources
                                .retain(|p| p.source_index as usize != idx);
                        }
                    }
                    crate::natives::SoundCommand::Destroy(h) => {
                        if let Some(idx) = crate::natives::ScriptHandleCodec::sound_source_index(h)
                        {
                            if let Some(src) = self.feedback.sound_sim.sources.get_mut(idx) {
                                src.active = false;
                            }
                            self.feedback.sound_sim.sources.delete(idx);
                            self.feedback
                                .sound_sim
                                .playing_sources
                                .retain(|p| p.source_index as usize != idx);
                        }
                    }
                    crate::natives::SoundCommand::PlayJingle(jingle) => {
                        self.feedback
                            .pending_side_effects
                            .sounds
                            .push(super::SoundCommand::Jingle(jingle));
                    }
                }
            }

            // ── Deferred game-logic commands ──
            // Re-entrant handlers are deferred until the first-pass effects
            // have released all temporary borrows.
            for cmd in deferred {
                match cmd {
                    crate::natives::DeferredCommand::AddAsSubordinateInitialize { chief } => {
                        let chief = self.entity_id_for_actor_handle(chief).unwrap_or_else(|| {
                            panic!(
                                "AddAsSubordinate lost its validated chief handle {chief} at the script barrier"
                            )
                        });
                        let scratch = self.build_owner_context_scratch_without_forecast(assets);
                        self.initialize_patrol_for_npc_from_owner_views(
                            assets,
                            chief,
                            &scratch.ai_entity_views,
                        );
                    }
                    crate::natives::DeferredCommand::RemoveAllSubordinates { actor } => {
                        if let Some(chief) = self.entity_id_for_actor_handle(actor) {
                            self.script_remove_all_subordinates(sim, assets, chief);
                        } else {
                            tracing::warn!(
                                "RemoveAllSubordinates ignored invalid chief handle {actor}"
                            );
                        }
                    }
                    crate::natives::DeferredCommand::SelectPC { actor, select } => {
                        // Scripted scene: targets the LOCAL seat.
                        if actor == 0 {
                            // NULL actor → select/deselect all
                            if select {
                                self.select_all_pcs(assets, 0);
                            } else {
                                self.unselect_all_pcs(0);
                            }
                        } else if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            if select {
                                // Script-path SelectPC uses `speak=false`
                                // — script already owns the sound flow.
                                self.select_pc(assets, 0, id, false, false);
                            } else {
                                self.players.seats[0].selection.retain(|&x| x != id);
                            }
                        }
                    }
                    crate::natives::DeferredCommand::StopActor { actor } => {
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            self.stop_owner(id, crate::sequence::SequencePriority::Script);
                        }
                    }
                    crate::natives::DeferredCommand::FreezeAll { freeze } => {
                        self.set_actors_frozen(freeze);
                    }
                    crate::natives::DeferredCommand::QuitSwordfight { actor } => {
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            self.quit_swordfight(sim, assets, id);
                        }
                    }
                    crate::natives::DeferredCommand::RemoveUnconsciousStars { actor } => {
                        // The titbit is only dropped when the actor is *not*
                        // currently unconscious — `remove_unconscious_stars_if`
                        // takes `is_still_unconscious` and short-circuits
                        // otherwise.  Read the live human-data flag now.
                        if let Some(id) = self.entity_id_for_actor_handle(actor)
                            && let Some(entity) = self.world.entities.get(id)
                        {
                            let still_unconscious =
                                entity.human_data().is_some_and(|h| h.unconscious);
                            self.feedback.titbit_manager.remove_unconscious_stars_if(
                                crate::titbit::ElementHandle(id.index()),
                                still_unconscious,
                            );
                        }
                    }
                    crate::natives::DeferredCommand::SetPlayable { actor, playable } => {
                        // PC playable state (pc.playable) was already set on
                        // the entity by the native call. Forward
                        // MSG_ENABLE/DISABLE_CHARACTER to the messenger
                        // carrying the actor's entity id so the handler
                        // can drop the PC from the selection and update
                        // Sherwood interface-hidden state.
                        let msg_type = if playable {
                            crate::messenger::PcMessage::EnableCharacter
                        } else {
                            crate::messenger::PcMessage::DisableCharacter
                        };
                        let pc_id = self.entity_id_for_actor_handle(actor);
                        self.orders.messenger.send(Message::pc(msg_type, pc_id));
                        tracing::debug!("SetPlayable: actor {actor} → playable={playable}");
                    }
                    cmd @ crate::natives::DeferredCommand::ProcessPatchEffects { .. } => {
                        post_script.push(cmd);
                    }
                    crate::natives::DeferredCommand::PutActorInBuilding { actor, building } => {
                        self.put_actor_in_building(actor, building);
                    }
                    crate::natives::DeferredCommand::ResetSpriteFrame { actor } => {
                        // Rewind the actor's sprite to frame 0 of its current row.
                        if let Some(id) = self.entity_id_for_actor_handle(actor)
                            && let Some(entity) = self.world.entities.get_mut(id)
                        {
                            entity.sprite_mut().reset_sprite_frame(false);
                        }
                    }
                    crate::natives::DeferredCommand::ClearAllQuickActionSlots { actor } => {
                        // Per-slot `SetQuickActionSequence(0, 0, i, 0xFFFFFFFF)`
                        // loop: drops QA titbits + clears macro_store slot.
                        if let Some(pc_id) = self.entity_id_for_actor_handle(actor) {
                            for slot in 0..crate::macro_store::NUMBER_OF_QA_MEMORY as u8 {
                                self.remove_quick_action_titbits_for(pc_id, slot);
                                if let Some(state) = self.players.macro_store.get_mut(pc_id) {
                                    state.clear_slot(slot as usize);
                                }
                            }
                        }
                    }
                    crate::natives::DeferredCommand::RelaunchPathAtNewSpeed { actor } => {
                        // From the `SetPathWalkingFlags` relaunch tail:
                        // re-issue GoTo at the freshly-changed walking
                        // flags so the speed change takes effect
                        // mid-segment instead of waiting for the next
                        // waypoint pickup.
                        if let Some(id) = self.entity_id_for_actor_handle(actor) {
                            self.relaunch_path_at_new_speed(sim, assets, id);
                        }
                    }
                }
            }

            for cmd in post_script {
                match cmd {
                    crate::natives::DeferredCommand::ProcessPatchEffects {
                        patch_index,
                        effects,
                    } => {
                        self.process_patch_effects(sim, assets, patch_index, effects);
                    }
                    _ => unreachable!("only ProcessPatchEffects is deferred post-script"),
                }
            }

            if !engine_commands.is_empty() {
                self.apply_host_commands(sim, assets, engine_commands);
            }

            // Sequence actions and effects emitted by this handler are
            // children of the current effect. Fully drive them with the same
            // active VM stack before exposing the detached parent tail.
            let child_result = self
                .drain_script_synchronous_actions(sim, assets, active_scripts)
                .and_then(|()| self.drain_script_effects_with_active(sim, assets, active_scripts));
            self.scripts
                .mission
                .as_mut()
                .expect("mission script vanished while restoring effect tail")
                .script_effects
                .restore_tail(parent_tail);
            child_result?;
        }
    }

    /// Load a mission script from the level directory.
    ///
    /// Looks up the pre-decoded script program in
    /// `assets.scripts.mission_programs` and installs it into
    /// `self.scripts.mission`.
    pub(crate) fn load_mission_script(&mut self, assets: &LevelAssets, scb_path: &std::path::Path) {
        let stem = scb_path.file_stem().and_then(|s| s.to_str());
        let program = stem.and_then(|name| {
            assets
                .scripts
                .mission_programs
                .get(name)
                .map(std::sync::Arc::clone)
        });
        let result = if let (Some(name), Some(program)) = (stem, program) {
            tracing::info!(
                "Mission script {}: loaded from LevelAssets",
                scb_path.display()
            );
            MissionScript::from_program(name.to_owned(), program)
        } else {
            Err(format!(
                "no mission script registered for {}",
                scb_path.display()
            ))
        };
        match result {
            Ok(script) => {
                tracing::info!(
                    "Loaded mission script: {} ({} classes)",
                    scb_path.display(),
                    script.manager.class_count(),
                );
                self.scripts.install_mission(script);
            }
            Err(e) => {
                tracing::warn!("Could not load mission script {}: {e}", scb_path.display());
            }
        }
    }

    /// Initialize the loaded mission script.
    ///
    /// Three-phase init:
    /// 1. **Per-waypoint binding** — for each waypoint in `hiking_paths`
    ///    with a script class, bind it and run `IWaypointScript::Initialize()`.
    /// 2. **Per-actor Initialize** — for each entity with a `script_class`,
    ///    create a temporary `ScriptInstance` bound to that class and call
    ///    its `Initialize()`.  Runs during entity loading.
    /// 3. **Global StartUp::Initialize(seed)** — the main mission script init.
    ///
    /// Called from `Engine::new` once the level loader has populated
    /// `assets.hiking_paths`.
    pub(crate) fn initialize_mission_script_with(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        seed: i32,
        hiking_paths: &[crate::level_data::RawHikingPath],
    ) {
        self.attach_script_bindings(assets);

        // Collect per-actor script classes before borrowing canonical entities.
        // Each actor with a script_class gets IActorScript::Initialize()
        // called during loading (before StartUp::Initialize).
        let per_actor_scripts: Vec<(i32, String)> = self
            .world
            .entities
            .actors()
            .filter_map(|(entity_id, entity)| {
                let script_class = &entity.actor_data()?.script_class;
                if script_class.is_empty() {
                    return None;
                }
                Some((
                    crate::natives::ScriptHandleCodec::actor_handle(entity_id),
                    script_class.clone(),
                ))
            })
            .collect();

        // Same collection pass for FX targets — each target with a
        // non-empty `script_class` gets its own `ScriptInstance`.
        // Each target carries its own VM and `Initialize()` runs
        // during `InitializeFromMissionStream`.
        let per_target_scripts: Vec<(i32, String)> = self
            .world
            .entities
            .targets()
            .filter_map(|(entity_id, target)| {
                if target.target.script_class.is_empty() {
                    return None;
                }
                Some((
                    crate::natives::ScriptHandleCodec::actor_handle(entity_id),
                    target.target.script_class.clone(),
                ))
            })
            .collect();

        // Scrolls also carry their own VMs — bind the class during
        // `InitializeFromMissionStream` and walk the list calling
        // `IScrollScript::Initialize()`.
        let per_scroll_scripts: Vec<(i32, String)> = self
            .world
            .entities
            .scrolls()
            .filter_map(|(entity_id, scroll)| {
                if scroll.script_class.is_empty() {
                    return None;
                }
                Some((
                    crate::natives::ScriptHandleCodec::actor_handle(entity_id),
                    scroll.script_class.clone(),
                ))
            })
            .collect();

        let _ = self.with_script_session(sim, assets, |script, script_domains, capabilities| {
            // ── Phase 1: Per-actor Initialize ──
            // Each actor's script class gets a ScriptInstance that persists for the
            // actor's lifetime — the heap (member variables) survives across calls
            // to Initialize, ActionChange, HandleEvent, FilterAIEvent, ProcessMessage.
            // Each VM receives a short-lived native context over the same
            // canonical capability bundle.
            let mut init_count = 0u32;
            for (handle, class_name) in &per_actor_scripts {
                if script.bind_actor(*handle, class_name, script_domains, capabilities) {
                    init_count += 1;
                }
            }
            if init_count > 0 {
                tracing::info!(
                    "Ran per-actor Initialize on {init_count} entities \
                     ({} instances persisted)",
                    script.actor_instances.len()
                );
            }

            // ── Phase 1b: Per-target Initialize ──
            // Run `IElementTargetScript::Initialize()` during
            // `InitializeFromMissionStream`.
            let mut target_init_count = 0u32;
            for (handle, class_name) in &per_target_scripts {
                if script.bind_target(*handle, class_name, script_domains, capabilities) {
                    target_init_count += 1;
                }
            }
            if target_init_count > 0 {
                tracing::info!(
                    "Ran per-target Initialize on {target_init_count} targets \
                     ({} instances persisted)",
                    script.target_instances.len()
                );
            }

            // ── Phase 1c: Per-scroll Initialize ──
            // Walk every scroll and run `IScrollScript::Initialize()`
            // on the bound class.
            let mut scroll_init_count = 0u32;
            for (handle, class_name) in &per_scroll_scripts {
                if script.bind_scroll(*handle, class_name, script_domains, capabilities) {
                    scroll_init_count += 1;
                }
            }
            if scroll_init_count > 0 {
                tracing::info!(
                    "Ran per-scroll Initialize on {scroll_init_count} scrolls \
                     ({} instances persisted)",
                    script.scroll_instances.len()
                );
            }

            // ── Phase 1d: Per-waypoint Initialize ──
            // For each scripted waypoint, call `Bind(class)` +
            // `IWaypointScript::Initialize()` during mission load.
            // Each waypoint is its own VM instance so the heap
            // persists across traversals.
            let mut wp_init_count = 0u32;
            for (path_idx, path) in hiking_paths.iter().enumerate() {
                for (wp_idx, wp) in path.waypoints.iter().enumerate() {
                    let crate::level_data::WaypointCommand::Script(ref class_name) = wp.command
                    else {
                        continue;
                    };
                    if class_name.is_empty() {
                        continue;
                    }
                    let Some(pid) = crate::ai::PathId::new(path_idx as u16) else {
                        continue;
                    };
                    if script.bind_waypoint(
                        pid,
                        wp_idx as u8,
                        class_name,
                        script_domains,
                        capabilities,
                    ) {
                        wp_init_count += 1;
                    }
                }
            }
            if wp_init_count > 0 {
                tracing::info!(
                    "Ran per-waypoint Initialize on {wp_init_count} waypoints \
                     ({} instances persisted)",
                    script.waypoint_instances.len()
                );
            }
        });

        for (handle, _) in &per_actor_scripts {
            if let Err(error) = self.call_script_vm(
                sim,
                assets,
                ScriptVmKey::Actor(*handle),
                "Initialize",
                &[],
                crate::natives::ScriptCallFrame::actor(*handle),
            ) {
                tracing::warn!("Actor Initialize (handle {handle}): {error}");
            }
        }
        for (handle, _) in &per_target_scripts {
            if let Err(error) = self.call_script_vm(
                sim,
                assets,
                ScriptVmKey::Target(*handle),
                "Initialize",
                &[],
                crate::natives::ScriptCallFrame::actor(*handle),
            ) {
                tracing::warn!("Target Initialize (handle {handle}): {error}");
            }
        }
        for (handle, _) in &per_scroll_scripts {
            let frame = crate::natives::ScriptCallFrame::default()
                .with_script_this(*handle)
                .with_current_scroll(*handle);
            if let Err(error) = self.call_script_vm(
                sim,
                assets,
                ScriptVmKey::Scroll(*handle),
                "Initialize",
                &[],
                frame,
            ) {
                tracing::warn!("Scroll Initialize (handle {handle}): {error}");
            }
        }
        for (path_idx, path) in hiking_paths.iter().enumerate() {
            for (wp_idx, waypoint) in path.waypoints.iter().enumerate() {
                if !matches!(
                    waypoint.command,
                    crate::level_data::WaypointCommand::Script(_)
                ) {
                    continue;
                }
                let Some(path_id) = crate::ai::PathId::new(path_idx as u16) else {
                    continue;
                };
                if let Err(error) = self.call_script_vm(
                    sim,
                    assets,
                    ScriptVmKey::Waypoint(path_id, wp_idx as u8),
                    "Initialize",
                    &[],
                    crate::natives::ScriptCallFrame::default(),
                ) {
                    tracing::warn!("Waypoint Initialize ({path_id}, {wp_idx}): {error}");
                }
            }
        }
        match self.call_script_vm(
            sim,
            assets,
            ScriptVmKey::Global,
            "Initialize",
            &[seed],
            crate::natives::ScriptCallFrame::default(),
        ) {
            Ok(value) => tracing::info!("Script StartUp::Initialize returned {value}"),
            Err(error) => tracing::warn!("Script StartUp::Initialize failed: {error}"),
        }

        // ── Mark AiControllers whose bound class overrides FilterAIEvent ──
        // Read by cascade `think()` sites in ai_enemy.rs to decide
        // whether to warn about the "would re-filter here, didn't"
        // divergence.  Unscripted NPCs leave the flag at its default
        // `false` and stay silent. This iteration reads the canonical engine
        // entity store directly.
        if let Some(script) = self.scripts.mission.as_ref() {
            let scripted_actors: Vec<i32> = script.actor_instances.keys().copied().collect();
            for handle in scripted_actors {
                let has_override = script.actor_has_function(handle, "FilterAIEvent");
                if !has_override {
                    continue;
                }
                let Some(id) = self.entity_id_for_actor_handle(handle) else {
                    continue;
                };
                if let Some(entity) = self.world.entities.get_mut(id)
                    && let Some(ai) = entity.ai_controller_mut()
                {
                    ai.has_script_filter_override = true;
                }
            }
        }

        // ── Phase 3: Zone script Initialize ──
        self.initialize_zone_scripts(sim, assets);

        // ── Phase 4: Populate initial zone occupants ──
        self.initialize_zone_occupants(assets);
    }

    /// Finalize the mission script (called on mission end).
    /// `abandoned` is true if the player quit/interrupted.
    pub(crate) fn finalize_mission_script(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        abandoned: bool,
    ) {
        if let Err(error) = self.call_script_vm(
            sim,
            assets,
            ScriptVmKey::Global,
            "Finalize",
            &[i32::from(abandoned)],
            crate::natives::ScriptCallFrame::default(),
        ) {
            tracing::warn!("Script Finalize failed: {error}");
        }
    }

    // ─── Per-actor script event dispatch ───────────────────────────

    /// Check one actor for an animation change and synchronously dispatch
    /// `ActionChange(newAction, oldAction)` to its per-actor script.
    pub(super) fn dispatch_actor_action_change_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: crate::element::EntityId,
    ) {
        let (new_action, old_action, handle) = {
            let entity = self.world.entities.get(entity_id).unwrap_or_else(|| {
                panic!(
                    "ActionChange creation slot {} lost typed entity {entity_id:?} before dispatch",
                    entity_id.index()
                )
            });
            let Some(actor) = entity.actor_data() else {
                return;
            };
            let handle = crate::natives::ScriptHandleCodec::actor_handle(entity_id);
            if !self
                .scripts
                .mission
                .as_ref()
                .is_some_and(|script| script.has_script_vm(ScriptVmKey::Actor(handle)))
            {
                return;
            }

            let new_action = self
                .orders
                .sequence_manager
                .current_order_for_actor(entity_id)
                .map(|(_, _, order)| order.order_type)
                .unwrap_or(crate::order::OrderType::NonanimationEnd);
            (new_action, actor.old_action, handle)
        };

        if new_action == old_action {
            return;
        }
        if let Err(error) = self.call_script_vm(
            sim,
            assets,
            ScriptVmKey::Actor(handle),
            "ActionChange",
            &[new_action as i32, old_action as i32],
            crate::natives::ScriptCallFrame::actor(handle),
        ) {
            tracing::warn!("ActionChange (handle {handle}): {error}");
        }

        // The callback may synchronously replace this actor's current order.
        // Match RHElementActor::Hourglass by rereading GetAnimation only after
        // ActionChange returns, then retain that live value for the next pass.
        self.world
            .entities
            .get(entity_id)
            .unwrap_or_else(|| {
                panic!(
                    "ActionChange callback for handle {handle} removed or replaced typed entity \
                     {entity_id:?} at creation slot {}",
                    entity_id.index()
                )
            })
            .actor_data()
            .unwrap_or_else(|| {
                panic!(
                    "ActionChange callback for handle {handle} changed typed actor \
                     {entity_id:?} into a non-actor"
                )
            });
        let live_action = self
            .orders
            .sequence_manager
            .current_order_for_actor(entity_id)
            .map(|(_, _, order)| order.order_type)
            .unwrap_or(crate::order::OrderType::NonanimationEnd);
        let entity = self.world.entities.get_mut(entity_id).unwrap_or_else(|| {
            panic!(
                "validated ActionChange actor {entity_id:?} for handle {handle} vanished before \
                 old_action retention at creation slot {}",
                entity_id.index()
            )
        });
        entity
            .actor_data_mut()
            .unwrap_or_else(|| {
                panic!(
                    "validated ActionChange actor {entity_id:?} for handle {handle} lost ActorData \
                     before old_action retention"
                )
            })
            .old_action = live_action;
    }

    /// Walk the live legacy element array in creation order and dispatch each
    /// scripted actor's animation change before advancing to the next slot.
    #[cfg(test)]
    pub(crate) fn dispatch_actor_action_changes(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        if self.scripts.mission.is_none() {
            return;
        }

        let mut slot = 0;
        while slot < self.world.entities.len() {
            if let Some(entity_id) = self.world.entities.id_at_legacy_slot(slot as u32) {
                self.dispatch_actor_action_change_for(sim, assets, entity_id);
            }
            slot += 1;
        }
    }

    /// Dispatch one scroll's due callback at its live legacy slot. The caller
    /// owns active checking and sprite advancement so callback mutations are
    /// visible before animation and before later slots.
    pub(super) fn dispatch_scroll_hourglass_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        scroll_id: EntityId,
    ) {
        const SCRIPT_HOURGLASS_TIMEOUT: u32 = 25;
        let handle = crate::natives::ScriptHandleCodec::actor_handle(scroll_id);
        let has_script = self
            .scripts
            .mission
            .as_ref()
            .is_some_and(|mission| mission.scroll_instances.contains_key(&handle));
        if !has_script {
            return;
        }
        let timeout = {
            let Entity::Scroll(scroll) =
                self.world.entities.get_mut(scroll_id).unwrap_or_else(|| {
                    panic!("scroll {scroll_id:?} vanished before its Hourglass timeout increment")
                })
            else {
                panic!("scroll Hourglass owner {scroll_id:?} is not Entity::Scroll")
            };
            scroll.script_hourglass_timeout += 1;
            scroll.script_hourglass_timeout
        };
        if timeout != SCRIPT_HOURGLASS_TIMEOUT {
            return;
        }
        let frame = crate::natives::ScriptCallFrame::default()
            .with_script_this(handle)
            .with_current_scroll(handle);
        if let Err(error) = self.call_script_vm(
            sim,
            assets,
            ScriptVmKey::Scroll(handle),
            "Hourglass",
            &[0],
            frame,
        ) {
            tracing::warn!("Scroll Hourglass (handle {handle}): {error}");
        }
        if let Some(entity) = self.world.entities.get_mut(scroll_id) {
            let Entity::Scroll(scroll) = entity else {
                panic!("scroll {scroll_id:?} changed concrete type during Hourglass callback")
            };
            scroll.script_hourglass_timeout = 0;
        }
    }

    /// Dispatch `IScrollScript::IsTaken(pc)` for a scroll being picked up.
    ///
    ///   1. Flip the scroll's sprite to `BonusThree` (the "opened
    ///      scroll" pose).
    ///   2. Call the bound script's `IsTaken(pc)` inside the
    ///      executing-scroll frame carried by [`ScriptVmKey::Scroll`].
    ///   3. If the script returns non-zero, mark the scroll `Taken`
    ///      and return `true`.  Otherwise `false` — the scroll keeps
    ///      the `Opened` visual but stays in-world.
    ///
    /// Scrolls without a bound script return `false` with no status
    /// change.
    ///
    /// NB: the scroll-pickup pipeline itself (PC ↔ scroll proximity,
    /// `Action::TakeScroll` dispatch) is not yet ported; this helper
    /// exists so whatever wires that up next can fire the
    /// script-bracketed `IsTaken` dispatch with a single call.
    pub fn scroll_is_taken(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        scroll_id: crate::element::EntityId,
        pc_id: crate::element::EntityId,
    ) -> bool {
        use crate::element::Entity;
        use ScrollStatus;

        let handle = crate::natives::ScriptHandleCodec::actor_handle(scroll_id);

        // Step 1 — always flip to the "opened" pose, even if there's no
        // script.  Set status to Opened and force the sprite animation
        // *before* the script-bound check.
        if let Some(Entity::Scroll(s)) = self.get_entity_mut(scroll_id) {
            let dir = s.element.direction() as u16;
            s.element
                .sprite
                .force_animation(crate::order::OrderType::BonusThree, dir);
        } else {
            tracing::warn!(?scroll_id, "scroll_is_taken: entity is not a scroll");
            return false;
        }
        self.set_scroll_status(scroll_id, ScrollStatus::Opened);

        // Step 2 — if no script is bound, return false immediately,
        // leaving the status at Opened.
        let has_script = self
            .scripts
            .mission
            .as_ref()
            .is_some_and(|ms| ms.scroll_instances.contains_key(&handle));
        if !has_script {
            return false;
        }

        // Step 3 — dispatch via the SetScrollExecutingScript bracket.
        let pc_handle = crate::natives::ScriptHandleCodec::actor_handle(pc_id);
        let result = self.call_script_vm(
            sim,
            assets,
            ScriptVmKey::Scroll(handle),
            "IsTaken",
            &[pc_handle],
            crate::natives::ScriptCallFrame::default()
                .with_script_this(handle)
                .with_current_scroll(handle),
        );

        let accepted = match result {
            Ok(v) => v != 0,
            Err(e) => {
                tracing::warn!("Scroll IsTaken (handle {handle}): {e}");
                false
            }
        };

        if accepted {
            // Flip the status to `Taken` and refresh the minimap dot
            // on a successful take.
            self.set_scroll_status(scroll_id, ScrollStatus::Taken);
        }
        accepted
    }

    pub(super) fn scroll_is_taken_in_script_driver(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        scroll_id: crate::element::EntityId,
        pc_id: crate::element::EntityId,
        active: &mut Vec<ActiveScriptCall>,
    ) -> Result<bool, ScriptDriverError> {
        use crate::element::Entity;

        let handle = crate::natives::ScriptHandleCodec::actor_handle(scroll_id);
        if let Some(Entity::Scroll(scroll)) = self.get_entity_mut(scroll_id) {
            let direction = scroll.element.direction() as u16;
            scroll
                .element
                .sprite
                .force_animation(crate::order::OrderType::BonusThree, direction);
        } else {
            tracing::warn!(?scroll_id, "scroll_is_taken: entity is not a scroll");
            return Ok(false);
        }
        self.set_scroll_status(scroll_id, ScrollStatus::Opened);
        let key = ScriptVmKey::Scroll(handle);
        let has_script = self
            .scripts
            .mission
            .as_ref()
            .is_some_and(|script| script.script_vm_has_function(key, "IsTaken"));
        if !has_script {
            return Ok(false);
        }
        let pc_handle = crate::natives::ScriptHandleCodec::actor_handle(pc_id);
        let accepted = self.call_script_vm_inner(
            sim,
            assets,
            key,
            "IsTaken",
            &[pc_handle],
            crate::natives::ScriptCallFrame::default()
                .with_script_this(handle)
                .with_current_scroll(handle),
            active,
        )? != 0;
        if accepted {
            self.set_scroll_status(scroll_id, ScrollStatus::Taken);
        }
        Ok(accepted)
    }

    // ─── Zone script system ───────────────────────────────────────

    /// Initialize per-zone script instances and call `Initialize()` on each.
    ///
    /// Creates `ScriptInstance`s for each script zone that has a `script_class`,
    /// runs `Initialize()`, and stores them in `MissionScript::zone_instances`.
    /// Called during mission init, after script sectors are registered on the grid.
    pub(crate) fn initialize_zone_scripts(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let classes: Vec<(usize, String)> = self
            .script_domains
            .zones
            .scripts
            .iter()
            .enumerate()
            .filter_map(|(zone_idx, zone_data)| {
                zone_data.script_associated.then(|| {
                    let name = zone_data.script_class_name.as_ref().unwrap_or_else(|| {
                        panic!("script-associated zone {zone_idx} has no class name")
                    });
                    (zone_idx, name.clone())
                })
            })
            .collect();

        if classes.is_empty() {
            return;
        }

        let bound: Vec<usize> = self
            .with_script_session(sim, assets, |script, _, _| {
                let mut bound = Vec::new();
                for (zone_idx, class_name) in &classes {
                let class_idx = match script.manager.find_class(&class_name) {
                    Some(idx) => idx,
                    None => panic!(
                        "Structural error in RHD: zone {zone_idx} references missing script class '{class_name}'"
                    ),
                };
                    let zone_inst = script.manager.create_instance_idx(class_idx);
                    script.zone_instances.insert(*zone_idx, zone_inst);
                    bound.push(*zone_idx);
                }
                bound
            })
            .expect("script zones require a loaded mission script");
        for zone_idx in &bound {
            self.call_script_vm(
                sim,
                assets,
                ScriptVmKey::Zone(*zone_idx),
                "Initialize",
                &[],
                crate::natives::ScriptCallFrame::default(),
            )
            .unwrap_or_else(|error| panic!("Zone Initialize ({zone_idx}) failed: {error}"));
        }
        if !bound.is_empty() {
            tracing::info!("Initialized {} zone scripts", bound.len());
        }
    }

    /// Scan all actors against all script-zone polygons and return the
    /// `(zone_idx, entity_idx, handle)` tuples for every actor that lies
    /// inside a zone.  Pure read helper — no state is mutated.
    ///
    /// Implements the "scan candidates → IsReallyInside" half of zone
    /// occupant initialization.  We walk every zone linearly rather
    /// than consulting a spatial index — same observable result
    /// (`contains_point` is bbox + polygon point-in-test), just
    /// O(actors × zones) rather than the per-cell narrowing the
    /// original used.  Documented perf gap.
    fn scan_zone_occupant_entries(
        &self,
        assets: &LevelAssets,
    ) -> Vec<(usize, crate::entity_id::EntityId, i32)> {
        let mut entries: Vec<(usize, crate::entity_id::EntityId, i32)> = Vec::new();
        if assets.scripts.zone_grid_indices.is_empty() {
            return entries;
        }
        for (actor_id, entity) in self.world.entities.actors() {
            let entity_id = actor_id.into();
            let ed = entity.element_data();
            // `in_honolulu` stands in for the `IsInside(GetBoxMap())`
            // reject — honolulu actors are parked off-map.  The extra
            // `!active` guard is a deliberate divergence; see the
            // `InitializeScriptSectorOccupants` parity entry.
            if !ed.active || ed.in_honolulu {
                continue;
            }
            let pos = ed.position_map();
            let layer = ed.layer();
            let handle = crate::natives::ScriptHandleCodec::actor_handle(actor_id);

            for (zone_idx, &grid_idx) in assets.scripts.zone_grid_indices.iter().enumerate() {
                // Skip zones that `DefineFlatTrajectoryZone` converted
                // into apex sectors — once converted, the SECTOR_SCRIPT
                // flag is dropped so the engine stops scanning them.
                if self
                    .script_domains
                    .zones
                    .scripts
                    .get(zone_idx)
                    .is_some_and(|z| z.transformed_to_apex)
                {
                    continue;
                }
                let gs = &self.world.fast_grid.level.sectors[grid_idx as usize];
                if gs.layer == layer && gs.contains_point(pos) {
                    entries.push((zone_idx, entity_id, handle));
                }
            }
        }
        // Carried-recursion: a PC entering a zone also recursively
        // enters its carried actor.  The polygon scan above normally
        // catches a sync'd carried, but when the carried is excluded
        // (in_honolulu / inactive at the moment of carry) we still
        // need it represented in the zone's occupants so the silent-
        // init path puts the carried in the right lists.
        let primary_len = entries.len();
        for i in 0..primary_len {
            let (zone_idx, eidx, _) = entries[i];
            let Some(entity) = self.world.entities.get(eidx) else {
                continue;
            };
            let Some(carried_id) = entity.pc_data().and_then(|pc| pc.carried) else {
                continue;
            };
            if entries
                .iter()
                .any(|&(z, e, _)| z == zone_idx && e == carried_id)
            {
                continue;
            }
            let carried_h = crate::natives::ScriptHandleCodec::actor_handle(carried_id);
            entries.push((zone_idx, carried_id, carried_h));
        }
        entries
    }

    /// Silent occupant population: pushes each entry into its zone's
    /// occupant list and applies the production work-icon, **without**
    /// firing any zone `EnterZone` script.  Matches the bare
    /// `AddOccupant` list-push semantics that never trigger scripts.
    fn apply_zone_occupant_entries(
        &mut self,
        entries: &[(usize, crate::entity_id::EntityId, i32)],
    ) {
        for &(zone_idx, entity_idx, _) in entries {
            self.script_domains.zones.scripts[zone_idx].add_occupant(entity_idx);
            let pt = self.script_domains.zones.scripts[zone_idx].production_sector_type;
            if pt != crate::sector_production::Type::Unknown {
                self.apply_production_work_icon(entity_idx, pt, true);
            }
        }
    }

    /// Bulk-clear occupant lists on every script zone.  Iterates
    /// script-sector objects and calls `RemoveAllOccupants` — no
    /// scripts fire.  Used by the post-mission Sherwood-entry refresh
    /// path, where occupant lists must be wiped before re-scanning
    /// against teleported positions.
    pub(crate) fn empty_all_script_sectors(&mut self) {
        for zone in &mut self.script_domains.zones.scripts {
            zone.remove_all_occupants();
        }
    }

    /// Clear every zone's occupant list and silently re-scan actor
    /// positions to rebuild it.  No `EnterZone` scripts fire.  Used
    /// to reconcile zone membership after post-mission teleports.
    pub(crate) fn refresh_zone_occupants_silent(&mut self, assets: &LevelAssets) {
        self.empty_all_script_sectors();
        if assets.scripts.zone_grid_indices.is_empty() {
            return;
        }
        let entries = self.scan_zone_occupant_entries(assets);
        self.apply_zone_occupant_entries(&entries);
    }

    /// Populate initial zone occupants by checking all actor positions
    /// against zone polygons without firing script callbacks.
    ///
    /// Original `RHFastFindGrid::InitializeScriptSectorOccupants` calls
    /// `RHSectorScript::AddOccupant`, which only appends to the list. Zone
    /// callbacks begin with later boundary crossings through `Enter`/`Leave`.
    pub(crate) fn initialize_zone_occupants(&mut self, assets: &LevelAssets) {
        if assets.scripts.zone_grid_indices.is_empty() {
            return;
        }

        let entries = self.scan_zone_occupant_entries(assets);
        if entries.is_empty() {
            return;
        }

        self.apply_zone_occupant_entries(&entries);

        tracing::info!(
            "Initialized {} zone occupant entries across {} zones",
            entries.len(),
            assets.scripts.zone_grid_indices.len()
        );
    }

    /// Dispatch script-sector transitions for the `LINE_SCRIPT` boundaries
    /// crossed by one actor move.
    ///
    /// This is the ordinary-movement counterpart of the Original's
    /// `RHElementActor::CheckForLineCrossing`. Polygon-wide reconciliation is
    /// reserved for exceptional relocation paths such as the post-flight
    /// update.
    pub(super) fn check_for_script_line_crossing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: crate::entity_id::EntityId,
        old_pos: crate::coordinates::MapPoint,
        new_pos: crate::coordinates::MapPoint,
        layer: u16,
    ) {
        if old_pos == new_pos {
            return;
        }
        let indices = self
            .world
            .fast_grid
            .get_crossing_script_line_indices(layer, old_pos, new_pos);
        if indices.is_empty() {
            return;
        }

        // Original removes a boundary when the actor's old position lies
        // exactly on it, then orders all remaining non-elevation crossings
        // by their intersection distance from the old position. Process every
        // crossed edge: unlike a net polygon-membership reconciliation,
        // RHElementActor::CheckForLineCrossing invokes Enter/Leave once per
        // LINE_SCRIPT object.
        let movement = crate::geo2d::segment(old_pos.to_geo(), new_pos.to_geo());
        let mut crossed: Vec<(f32, crate::fast_find_grid::LineIndex)> = indices
            .into_iter()
            .filter_map(|line_index| {
                let line = &self.world.fast_grid.level.lines[usize::from(line_index)];
                let old_dx = old_pos.x - line.a.x;
                let old_dy = old_pos.y - line.a.y;
                let line_dx = line.b.x - line.a.x;
                let line_dy = line.b.y - line.a.y;
                if line_dx * old_dy - line_dy * old_dx == 0.0 {
                    return None;
                }
                let point = crate::geo2d::segment_intersection(movement, line.segment()).point()?;
                let dx = point.x - old_pos.x;
                let dy = point.y - old_pos.y;
                Some((dx * dx + dy * dy, line_index))
            })
            .collect();
        crossed.sort_by(|(left, _), (right, _)| left.total_cmp(right));

        for (_, line_index) in crossed {
            let Some(zone_idx) = self.world.fast_grid.level.lines[usize::from(line_index)]
                .script_zone_index
                .map(usize::from)
            else {
                // RHLineScript::Cross is intentionally empty in the Original.
                continue;
            };
            if self.script_domains.zones.scripts[zone_idx].transformed_to_apex {
                continue;
            }
            let grid_idx = *assets
                .scripts
                .zone_grid_indices
                .get(zone_idx)
                .unwrap_or_else(|| panic!("script line references missing zone {zone_idx}"));
            let inside =
                self.world.fast_grid.level.sectors[grid_idx as usize].contains_point(new_pos);
            self.dispatch_script_zone_crossing(sim, assets, zone_idx, entity_id, inside);
        }
    }

    fn dispatch_script_zone_crossing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        zone_idx: usize,
        entity_id: crate::entity_id::EntityId,
        entering: bool,
    ) {
        let (script_associated, production_type) = {
            let zone = &mut self.script_domains.zones.scripts[zone_idx];
            if entering {
                zone.enter(entity_id);
            } else {
                zone.leave(entity_id);
            }
            (zone.script_associated, zone.production_sector_type)
        };

        if script_associated {
            let method = if entering { "EnterZone" } else { "ExitZone" };
            let handle = crate::natives::ScriptHandleCodec::actor_handle(entity_id);
            if let Err(error) = self.call_script_vm(
                sim,
                assets,
                ScriptVmKey::Zone(zone_idx),
                method,
                &[handle],
                crate::natives::ScriptCallFrame::default(),
            ) {
                tracing::warn!("Zone {zone_idx} {method} (actor {handle}): {error}");
            }
        }

        // Original recursively crosses the carried actor only after the
        // carrier's callback, and applies the carrier's work icon last.
        let carried = self
            .world
            .entities
            .get(entity_id)
            .and_then(|entity| entity.pc_data())
            .and_then(|pc| pc.carried);
        if let Some(carried) = carried {
            self.dispatch_script_zone_crossing(sim, assets, zone_idx, carried, entering);
        }

        if production_type != crate::sector_production::Type::Unknown {
            self.apply_production_work_icon(entity_id, production_type, entering);
        }
    }

    /// Reconcile one actor against script-sector polygons after a flight.
    ///
    /// Flights do not traverse ordinary movement lines. The Original handles
    /// this exceptional relocation in `RHElementActor::
    /// UpdateScriptSectorsAfterFlight`, checking layer, owning motion-sector
    /// number, and polygon containment for the landed actor only.
    pub(super) fn update_script_sectors_after_flight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let (position, layer, sector) = {
            let entity = self
                .world
                .entities
                .get(entity_id)
                .unwrap_or_else(|| panic!("landed actor {entity_id} is missing"));
            let data = entity.element_data();
            (data.position_map(), data.layer(), data.sector())
        };

        for (zone_idx, &grid_idx) in assets.scripts.zone_grid_indices.iter().enumerate() {
            if self.script_domains.zones.scripts[zone_idx].transformed_to_apex {
                continue;
            }
            let grid_sector = self
                .world
                .fast_grid
                .level
                .sectors
                .get(grid_idx as usize)
                .unwrap_or_else(|| panic!("script zone {zone_idx} references missing grid sector"));
            let was_inside = self.script_domains.zones.scripts[zone_idx].is_inside(entity_id);
            let is_inside = grid_sector.layer == layer
                && sector.map(i16::from) == Some(grid_sector.sector_number.get())
                && grid_sector.contains_point(position);
            if was_inside != is_inside {
                self.dispatch_script_zone_crossing(sim, assets, zone_idx, entity_id, is_inside);
            }
        }
    }

    /// Set a PC's work icon when entering/leaving a script sector with a
    /// production type.
    pub(super) fn apply_production_work_icon(
        &mut self,
        entity_id: EntityId,
        production_type: crate::sector_production::Type,
        entering: bool,
    ) {
        use crate::element::WorkIcon;
        use crate::sector_production::Type as PT;

        let Some(entity) = self.world.entities.get_mut(entity_id) else {
            return;
        };
        let crate::engine::Entity::Pc(pc) = entity else {
            return;
        };

        if entering {
            // Map production type onto the WorkIcon enum. Relic / Unknown have
            // no icon (work icons cover types 0..11; Relic=12 falls through
            // at the call site).
            let icon = match production_type {
                PT::MakeArrow => WorkIcon::Arrows,
                PT::MakePurse => WorkIcon::Purses,
                PT::MakeStone => WorkIcon::Stones,
                PT::MakeApple => WorkIcon::Apples,
                PT::MakeAle => WorkIcon::Beer,
                PT::MakeLamblegg => WorkIcon::Legs,
                PT::MakePlant => WorkIcon::Plants,
                PT::MakeNet => WorkIcon::Nets,
                PT::MakeWaspNest => WorkIcon::Wasps,
                PT::TrainBow => WorkIcon::BowTraining,
                PT::TrainHandToHand => WorkIcon::SwordTraining,
                PT::Heal => WorkIcon::Regeneration,
                PT::Relic | PT::Unknown => return,
            };
            pc.pc.work_icon = icon;
        } else {
            pc.pc.work_icon = WorkIcon::None;
        }
    }

    // ─── Sequence SendMessage → ProcessMessage dispatch ──────────

    /// Extract message properties from a generic sequence element.
    /// Returns `(message, argument, extended_argument)`.
    pub(super) fn extract_message_properties(
        &self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> (i32, i32, i32) {
        use crate::sequence::{Field, FieldValue};
        let elem = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .unwrap_or_else(|| panic!("missing SendMessage element {seq_id:?}/{elem_idx}"));
        let msg = match elem.get_property(Field::Message) {
            Some(FieldValue::Integer(v)) => *v as i32,
            other => panic!("SendMessage {seq_id:?}/{elem_idx} has malformed Message: {other:?}"),
        };
        let arg1 = match elem.get_property(Field::MessageArgument) {
            Some(FieldValue::Integer(v)) => *v as i32,
            other => {
                panic!("SendMessage {seq_id:?}/{elem_idx} has malformed MessageArgument: {other:?}")
            }
        };
        let arg2 = match elem.get_property(Field::MessageExtendedArgument) {
            Some(FieldValue::Integer(v)) => *v as i32,
            other => panic!(
                "SendMessage {seq_id:?}/{elem_idx} has malformed MessageExtendedArgument: {other:?}"
            ),
        };
        (msg, arg1, arg2)
    }

    /// Dispatch deferred `ProcessMessage` calls from sequence SendMessage
    /// elements.
    ///
    /// Per-actor messages go to the actor's script `ProcessMessage(msg, arg1, arg2)`.
    /// EngineInner-level messages (ownerless) go to the global StartUp script's
    /// `ProcessMessage`.
    ///
    /// Routes through `IEngineScript::ProcessMessage` /
    /// `IActorScript::ProcessMessage`.
    pub(super) fn dispatch_sequence_messages(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        per_actor: &[(i32, i32, i32, i32)],
        engine_level: &[(i32, i32, i32)],
    ) {
        for &(handle, msg, arg1, arg2) in per_actor {
            if let Err(error) = self.call_script_vm(
                sim,
                assets,
                ScriptVmKey::Actor(handle),
                "ProcessMessage",
                &[msg, arg1, arg2],
                crate::natives::ScriptCallFrame::actor(handle),
            ) {
                tracing::warn!("Sequence ProcessMessage (actor {handle}, msg {msg}): {error}");
            }
        }

        for &(msg, arg1, arg2) in engine_level {
            if let Err(error) = self.call_script_vm(
                sim,
                assets,
                ScriptVmKey::Global,
                "ProcessMessage",
                &[msg, arg1, arg2],
                crate::natives::ScriptCallFrame::default(),
            ) {
                tracing::warn!("EngineInner ProcessMessage(msg {msg}): {error}");
            }
        }
    }

    /// Send a one-shot engine-level `ProcessMessage` to the global
    /// StartUp script.
    ///
    /// Used e.g. by the Sherwood `GoToExit` button (msg=1000).  Thin
    /// wrapper over the existing `dispatch_sequence_messages`
    /// engine-level path.
    pub(crate) fn dispatch_startup_message(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        msg: i32,
        arg1: i32,
        arg2: i32,
    ) {
        self.dispatch_sequence_messages(sim, assets, &[], &[(msg, arg1, arg2)]);
    }

    // ─── AI event filter precompute ─────────────────────────────

    /// Run the per-actor `FilterAIEvent` for a stimulus about to be
    /// dispatched to `handle` (opaque script actor handle).
    ///
    /// Returns `true` if `think()` should proceed, `false` if the
    /// script blocked the stimulus.  Implements the early-gate:
    ///
    /// ```text
    /// SetScriptThis(self);
    /// ok = (FilterAIEvent(stimulus_actor, event_code) != 0);
    /// SetScriptThis(prev);
    /// if (!ok) { register_log(LOG_EVENT_REFUSED, 0); return false; }
    /// ```
    ///
    /// Callers must invoke this *before* acquiring a `&mut` borrow on
    /// the target entity, since the script session leases
    /// `self.world.entities` for the callback. The function is a
    /// no-op (returns `true`) for:
    ///  - Actors with no script instance or no `FilterAIEvent`
    ///    override (the base-class `FilterAIEvent` returns 1 / allow).
    ///  - Script VM errors — logged and treated as allow so a
    ///    script bug never blocks AI progress.
    ///
    /// Source actor is extracted from `stimulus.info`: `Human(h)` becomes
    /// a script actor handle; other info variants become 0 (originally NULL).
    pub fn filter_stimulus(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        handle: i32,
        stimulus: &crate::ai::Stimulus,
    ) -> bool {
        if !sim.config().script_enabled {
            return true;
        }
        let is_scripted = self
            .entity_id_for_actor_handle(handle)
            .and_then(|id| self.world.entities.get(id))
            .and_then(Entity::actor_data)
            .is_some_and(|actor| !actor.script_class.is_empty());
        if !is_scripted {
            return true;
        }

        // Original: RHArtificialIntelligence::StartThink assigns -2 in the
        // default switch arm and still calls FilterAIEvent for scripted NPCs.
        let code = crate::ai::stimulus_to_ai_event_code(stimulus.stimulus_type).unwrap_or(-2);

        let source = match stimulus.info {
            crate::ai::StimulusInfo::Human(h) => crate::natives::ScriptHandleCodec::actor_handle(
                crate::element::EntityId::Soldier(crate::entity_id::SoldierId(h)),
            ),
            _ => 0,
        };

        // Fast paths that skip script dispatch.
        let has_override = match self.scripts.mission.as_ref() {
            Some(s) => s.actor_has_function(handle, "FilterAIEvent"),
            None => return true,
        };
        if !has_override {
            return true;
        }

        let result = self.call_script_vm(
            sim,
            assets,
            ScriptVmKey::Actor(handle),
            "FilterAIEvent",
            &[source, code],
            crate::natives::ScriptCallFrame::actor(handle),
        );

        match result {
            Ok(v) => v != 0,
            Err(e) => {
                tracing::warn!(
                    "FilterAIEvent(handle={handle}, source={source}, code={code}) failed: {e} — allowing"
                );
                true
            }
        }
    }

    /// Run [`filter_stimulus`](Self::filter_stimulus) on `stimulus` for
    /// the AI on `entity_id`, and dispatch to `think()` if the filter
    /// allows it.  Returns `think()`'s handled-bool — returns `false`
    /// when the filter blocks.  Also returns `false` when the entity
    /// has no AI controller (nothing to think with).
    ///
    /// This is the canonical entry point for engine-layer stimulus
    /// dispatch — every external stimulus (detection pass, command
    /// completion, reach-point, etc.) should route through here so
    /// `FilterAIEvent` fires live with the actual source.
    ///
    /// Cascades — `self.think(&other_stimulus, ...)` calls inside
    /// `EnemyAi::think` / `FriendlyAi::think` — intentionally do *not*
    /// go through this path.  `think()` doesn't have engine access;
    /// routing cascades through a deferred queue would break the
    /// synchronous-within-tick semantics the script runtime relies on.
    /// Audit of the shipped `fullgame` `.scb` content confirmed no
    /// script filters any cascade-emitted stimulus, so the divergence
    /// is harmless for shipped content.  A warning is logged in
    /// `EnemyAi::think_*` cascades if this assumption ever breaks.
    pub(crate) fn dispatch_filtered_stimulus(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        tick_data: &crate::ai::AiPerTickData,
    ) -> bool {
        self.dispatch_filtered_stimulus_with_owner_mode(
            sim,
            assets,
            entity_id,
            stimulus,
            ctx,
            Some(tick_data),
            false,
        )
    }

    pub(super) fn dispatch_filtered_stimulus_without_forecast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        tick_data: &crate::ai::AiPerTickData,
    ) -> bool {
        self.dispatch_filtered_stimulus_with_owner_mode(
            sim,
            assets,
            entity_id,
            stimulus,
            ctx,
            Some(tick_data),
            true,
        )
    }

    pub(super) fn dispatch_filtered_friendly_stimulus_without_forecast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
    ) -> bool {
        self.dispatch_filtered_stimulus_with_owner_mode(
            sim, assets, entity_id, stimulus, ctx, None, true,
        )
    }

    fn dispatch_filtered_stimulus_with_owner_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        enemy_tick_data: Option<&crate::ai::AiPerTickData>,
        owner_local_no_forecast: bool,
    ) -> bool {
        let handle = crate::natives::ScriptHandleCodec::actor_handle(entity_id);
        if !self.filter_stimulus(sim, assets, handle, stimulus) {
            return false;
        }
        // Hoist the canonical door slice before grabbing the mutable
        // entity borrow — the friendly AI's `alert_soldier` needs it for the
        // `ALERTFLAG_CHECK_DOOR_PATH` retry.
        let doors = self.script_domains.interactables.doors.as_slice();
        let friendly_tick = self
            .world
            .entities
            .get(entity_id)
            .is_some_and(|entity| {
                matches!(entity, Entity::Civilian(c) if c.npc.ai_brain.friendly().is_some())
            })
            .then(|| self.build_friendly_tick_data_without_forecasts(entity_id));
        let handled = {
            let ai_global = &mut self.ai.global;
            let Some(entity) = self.world.entities.get_mut(entity_id) else {
                return false;
            };
            if let Some(enemy_ai) = entity.enemy_ai_mut() {
                enemy_ai.think(
                    sim,
                    stimulus,
                    ai_global,
                    ctx,
                    enemy_tick_data.unwrap_or_else(|| {
                        panic!(
                            "filtered Enemy AI stimulus for owner {} requires typed enemy tick data",
                            entity_id.index()
                        )
                    }),
                    Some(&self.world.fast_grid),
                )
            } else if let Some(friendly_ai) = entity.friendly_ai_mut() {
                friendly_ai.think(
                    sim,
                    stimulus,
                    ai_global,
                    ctx,
                    &friendly_tick.unwrap_or_else(|| {
                        panic!(
                            "filtered Friendly AI stimulus for owner {} requires truthful friendly tick data",
                            entity_id.index()
                        )
                    }),
                    Some(&self.world.fast_grid),
                    Some(doors),
                )
            } else {
                return false;
            }
        };

        // ReconsiderSwordfight calls ProposeGoodSwordStrike before returning
        // to its caller. Keep that event-owned RNG and sequence work ahead of
        // later actors instead of leaving the one-shot authorization for the
        // global melee maintenance pass.
        self.consume_pending_enemy_sword_attack_for(sim, assets, entity_id);

        // `SetState` calls FilterAIEvent before any of the caller's deferred
        // effects. The entity borrow above is the first point at which the
        // engine can safely re-enter the actor VM.
        self.drain_ai_owner_work_for_mode(sim, assets, entity_id, owner_local_no_forecast);
        handled
    }

    /// Drain one AI owner's queued `SetState` callbacks in FIFO order.
    ///
    /// Every queue entry is consumed even when scripts are disabled, no
    /// mission is installed, the actor VM is unbound, or its class inherits
    /// the default `FilterAIEvent`. Callback return values are informational
    /// and deliberately ignored. A callback may append more transitions to
    /// this same owner; those are observed without taking the whole queue.
    /// The temporary outgoing restore covers only base state/substate: an
    /// Enemy SetState tail already applied before this callback remains visible.
    /// Friendly alert is intentionally pre-callback, matching Original.
    pub(crate) fn drain_ai_owner_work_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: crate::element::EntityId,
    ) {
        self.drain_ai_owner_work_for_mode(sim, assets, owner, false);
    }

    pub(super) fn drain_ai_owner_work_for_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: crate::element::EntityId,
        owner_local_no_forecast: bool,
    ) {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum OwnerAiKind {
            Enemy,
            Friendly,
        }

        const MAX_OWNER_WORK: usize = 128;
        let handle = crate::natives::ScriptHandleCodec::actor_handle(owner);

        for work_index in 0..MAX_OWNER_WORK {
            let work = {
                let Some(entity) = self.world.entities.get_mut(owner) else {
                    if work_index == 0 {
                        return;
                    }
                    panic!(
                        "AI owner-work recipient {} disappeared before item {}",
                        owner.index(),
                        work_index
                    );
                };
                let Some(ai) = entity.ai_controller_mut() else {
                    if work_index == 0 {
                        return;
                    }
                    panic!(
                        "AI owner-work recipient {} lost its AI before item {}",
                        owner.index(),
                        work_index
                    );
                };
                if ai.outbox.reentrant.owner_work.is_empty() {
                    return;
                }
                ai.outbox.reentrant.owner_work.remove(0)
            };

            let notification = match work {
                crate::ai::AiOwnerWork::StateChange(notification) => notification,
                crate::ai::AiOwnerWork::ResumeGotoRouteReachPoint {
                    owner_boundary_positions,
                } => {
                    self.resume_goto_route_reach_point_for_npc(
                        sim,
                        owner,
                        assets,
                        &owner_boundary_positions,
                    );
                    continue;
                }
                crate::ai::AiOwnerWork::Speech(attempt) => {
                    // A rejected Say invokes MYTALK synchronously before Say
                    // returns. Detach the outer statement tail so recursive
                    // Think work and its logs settle ahead of that tail.
                    let later_work = self
                        .world
                        .entities
                        .get_mut(owner)
                        .and_then(Entity::ai_controller_mut)
                        .map(|ai| std::mem::take(&mut ai.outbox.reentrant.owner_work))
                        .unwrap_or_else(|| {
                            panic!("speech owner {} vanished before settlement", owner.index())
                        });
                    let settlement = self.settle_npc_speech_attempt(assets, owner, attempt);
                    if settlement.invoke_finished_callback {
                        if owner_local_no_forecast {
                            self.drain_self_stimuli_for_npc_without_forecast(sim, owner, assets);
                        } else {
                            self.drain_self_stimuli_for_npc(sim, owner, assets);
                        }
                    }
                    if let Some(finalization) = settlement.category_rejection {
                        self.finalize_category_speech_rejection(owner, finalization);
                    }
                    self.world
                        .entities
                        .get_mut(owner)
                        .and_then(Entity::ai_controller_mut)
                        .unwrap_or_else(|| {
                            panic!("speech owner {} vanished after settlement", owner.index())
                        })
                        .outbox
                        .reentrant
                        .owner_work
                        .extend(later_work);
                    continue;
                }
                crate::ai::AiOwnerWork::RestoreDetectableObjects {
                    knocked_out_in_money_fight,
                } => {
                    self.restore_detectable_objects_for_npc(owner, knocked_out_in_money_fight);
                    continue;
                }
                crate::ai::AiOwnerWork::InformResurrection => {
                    self.broadcast_resurrection(owner);
                    continue;
                }
                crate::ai::AiOwnerWork::LaunchTimer {
                    frames,
                    current_frame,
                } => {
                    self.world
                        .entities
                        .get_mut(owner)
                        .and_then(Entity::ai_controller_mut)
                        .unwrap_or_else(|| {
                            panic!("timer owner {} vanished before settlement", owner.index())
                        })
                        .launch_timer(frames, current_frame);
                    continue;
                }
                crate::ai::AiOwnerWork::SetEyeStatus(status) => {
                    let npc = self
                        .world
                        .entities
                        .get_mut(owner)
                        .and_then(Entity::npc_data_mut)
                        .unwrap_or_else(|| {
                            panic!(
                                "eye-status owner {} vanished before settlement",
                                owner.index()
                            )
                        });
                    crate::ai_vision::set_view_status(npc, status);
                    continue;
                }
            };

            // Work produced by a FilterAIEvent callback belongs inside this
            // SetState call and therefore precedes statements the outer
            // pure-Rust handler queued after SetState. Detach that later tail
            // while the VM runs, then splice recursively produced work ahead
            // of it.
            let (
                owner_kind,
                is_scripted,
                caller_tail_state,
                later_work,
                later_actor_effects,
                later_self_stimuli,
                later_cross_npc_actions,
            ) = {
                let entity = self.world.entities.get_mut(owner).unwrap_or_else(|| {
                    panic!(
                        "AI SetState owner {} disappeared before callback {}",
                        owner.index(),
                        work_index
                    )
                });
                let owner_kind = match entity {
                    Entity::Soldier(s)
                        if matches!(&s.npc.ai_brain, crate::element::AiBrain::Enemy(_)) =>
                    {
                        OwnerAiKind::Enemy
                    }
                    Entity::Soldier(_) => {
                        panic!("AI SetState owner {} has a non-Enemy brain", owner.index())
                    }
                    Entity::Civilian(c)
                        if matches!(&c.npc.ai_brain, crate::element::AiBrain::Friendly(_)) =>
                    {
                        OwnerAiKind::Friendly
                    }
                    Entity::Civilian(_) => panic!(
                        "AI SetState owner {} has a non-Friendly brain",
                        owner.index()
                    ),
                    other => panic!(
                        "AI SetState owner {} drifted to invalid kind {:?}",
                        owner.index(),
                        other.element_data().kind
                    ),
                };
                let ai = entity
                    .ai_controller_mut()
                    .unwrap_or_else(|| panic!("AI SetState owner {} lost its AI", owner.index()));
                // The pure-Rust caller continues after SetState returns before
                // this deferred callback can run. Preserve any later direct
                // state assignment made by that caller (for example,
                // ExecuteNextMacroCommand immediately completing an empty
                // restored macro after first entering DefaultInMacro).
                let caller_tail_state = (ai.current_state, ai.current_substate);
                let later_work = std::mem::take(&mut ai.outbox.reentrant.owner_work);
                let (later_actor_effects, later_self_stimuli, later_cross_npc_actions) =
                    if let Some(prefix) = notification.actor_effects_before_callback.clone() {
                        (
                            Some(std::mem::replace(&mut ai.outbox.actor, prefix)),
                            Some(std::mem::take(&mut ai.outbox.reentrant.self_stimuli)),
                            Some(std::mem::take(&mut ai.outbox.reentrant.cross_npc_actions)),
                        )
                    } else {
                        (None, None, None)
                    };
                let is_scripted = entity
                    .actor_data()
                    .is_some_and(|actor| !actor.script_class.is_empty());
                (
                    owner_kind,
                    is_scripted,
                    caller_tail_state,
                    later_work,
                    later_actor_effects,
                    later_self_stimuli,
                    later_cross_npc_actions,
                )
            };

            // Effects issued before SetState are already inside the Original
            // call stack. Settle that prefix while keeping the pure-Rust
            // caller tail detached from the synchronous script callback.
            if later_actor_effects.is_some() {
                self.drain_pending_for_npc_mode(sim, owner, assets, owner_local_no_forecast, false);
            }

            let source = match notification.source {
                crate::ai::AiStateChangeSource::SelfActor => handle,
                crate::ai::AiStateChangeSource::Null => 0,
                crate::ai::AiStateChangeSource::Human(raw_index) => {
                    crate::natives::ScriptHandleCodec::actor_handle_from_index(raw_index as usize)
                }
            };
            let code = notification.incoming_state.state_change_event_code();
            let should_call = is_scripted
                && sim.config().script_enabled
                && self
                    .scripts
                    .mission
                    .as_ref()
                    .is_some_and(|script| script.actor_has_function(handle, "FilterAIEvent"));
            if !should_call {
                // With no observable synchronous callback, consuming the
                // deferred notification must not rewind canonical AI state.
                // The pure-Rust handler may already have performed a later
                // direct state mutation after SetState returned (Original's
                // one-point macro completion does this before re-entering
                // EVENT_REACHPOINT).
                let ai = self
                    .world
                    .entities
                    .get_mut(owner)
                    .and_then(Entity::ai_controller_mut)
                    .unwrap_or_else(|| {
                        panic!(
                            "AI SetState owner {} vanished while consuming callback {}",
                            owner.index(),
                            work_index
                        )
                    });
                ai.outbox.reentrant.owner_work.extend(later_work);
                if let Some(later_actor_effects) = later_actor_effects {
                    ai.outbox
                        .reentrant
                        .self_stimuli
                        .extend(later_self_stimuli.expect("isolated SetState self-stimulus tail"));
                    ai.outbox
                        .reentrant
                        .cross_npc_actions
                        .extend(later_cross_npc_actions.expect("isolated SetState cross-NPC tail"));
                    debug_assert!(
                        !ai.outbox.actor.has_boundary_work(),
                        "non-scripted SetState prefix left undrained actor effects"
                    );
                    ai.outbox.actor = later_actor_effects;
                }
                continue;
            }

            {
                let ai = self
                    .world
                    .entities
                    .get_mut(owner)
                    .and_then(Entity::ai_controller_mut)
                    .unwrap_or_else(|| {
                        panic!(
                            "AI SetState owner {} vanished before callback rewind {}",
                            owner.index(),
                            work_index
                        )
                    });
                ai.set_ai_state(notification.outgoing_state);
                ai.current_substate = notification.outgoing_substate;
            }
            #[cfg(test)]
            {
                let entity = self.world.entities.get(owner).unwrap_or_else(|| {
                    panic!(
                        "AI SetState owner {} vanished before observation",
                        owner.index()
                    )
                });
                let npc = entity.npc_data().unwrap_or_else(|| {
                    panic!(
                        "AI SetState owner {} lost NPC data before observation",
                        owner.index()
                    )
                });
                let timer_is_running = entity
                    .ai_controller()
                    .unwrap_or_else(|| {
                        panic!(
                            "AI SetState owner {} lost AI before observation",
                            owner.index()
                        )
                    })
                    .timer_is_running;
                let body_references_to_owner = self
                    .world
                    .entities
                    .npcs()
                    .filter(|(_, entity)| {
                        entity
                            .npc_data()
                            .and_then(|npc| {
                                npc.detectable_lists
                                    .get(crate::element::DetectableType::Body as usize)
                            })
                            .is_some_and(|bodies| {
                                bodies
                                    .iter()
                                    .any(|detectable| detectable.element == Some(owner))
                            })
                    })
                    .count();
                AI_STATE_CALLBACK_OBSERVATIONS.with(|observations| {
                    if let Some(observations) = observations.borrow_mut().as_mut() {
                        observations.push(AiStateCallbackObservation {
                            owner,
                            eye_status: npc.eye_status,
                            timer_is_running,
                            body_references_to_owner,
                        });
                    }
                });
            }
            if let Err(error) = self.call_script_vm(
                sim,
                assets,
                ScriptVmKey::Actor(handle),
                "FilterAIEvent",
                &[source, code],
                crate::natives::ScriptCallFrame::actor(handle),
            ) {
                tracing::warn!(
                    actor_handle = handle,
                    source_handle = source,
                    event_code = code,
                    %error,
                    "AI SetState FilterAIEvent callback failed"
                );
            }
            // Native calls made by FilterAIEvent are still inside SetState
            // and therefore observe the outgoing pair. Close callback-local
            // recursive stimuli before committing the incoming pair.
            if later_actor_effects.is_some() {
                self.drain_self_stimuli_for_npc_without_forecast(sim, owner, assets);
            }

            let entity = self.world.entities.get_mut(owner).unwrap_or_else(|| {
                panic!(
                    "AI SetState owner {} disappeared during callback {} ({:?} -> {:?})",
                    owner.index(),
                    work_index,
                    notification.outgoing_state,
                    notification.incoming_state
                )
            });
            let ai = match owner_kind {
                OwnerAiKind::Enemy => {
                    &mut entity
                        .enemy_ai_mut()
                        .unwrap_or_else(|| {
                            panic!(
                                "AI SetState owner {} lost EnemyAi during callback {}",
                                owner.index(),
                                work_index
                            )
                        })
                        .base
                }
                OwnerAiKind::Friendly => {
                    &mut entity
                        .friendly_ai_mut()
                        .unwrap_or_else(|| {
                            panic!(
                                "AI SetState owner {} lost FriendlyAi during callback {}",
                                owner.index(),
                                work_index
                            )
                        })
                        .base
                }
            };
            ai.set_ai_state(notification.incoming_state);
            ai.current_substate = notification.incoming_substate;
            if caller_tail_state != (notification.incoming_state, notification.incoming_substate) {
                // Original has already returned from this SetState and run
                // these caller-tail assignments by now. Reapply the captured
                // canonical pair after the callback's outgoing→incoming
                // transaction instead of letting the deferred transaction
                // rewind newer state.
                ai.set_ai_state(caller_tail_state.0);
                ai.current_substate = caller_tail_state.1;
            }

            let ai = self
                .world
                .entities
                .get_mut(owner)
                .and_then(Entity::ai_controller_mut)
                .unwrap_or_else(|| {
                    panic!(
                        "AI SetState owner {} vanished after callback settlement {}",
                        owner.index(),
                        work_index
                    )
                });
            ai.outbox.reentrant.owner_work.extend(later_work);
            if let Some(later_actor_effects) = later_actor_effects {
                ai.outbox
                    .reentrant
                    .self_stimuli
                    .extend(later_self_stimuli.expect("isolated SetState self-stimulus tail"));
                ai.outbox
                    .reentrant
                    .cross_npc_actions
                    .extend(later_cross_npc_actions.expect("isolated SetState cross-NPC tail"));
                debug_assert!(
                    !ai.outbox.actor.has_boundary_work(),
                    "SetState callback left actor effects outside its synchronous barrier"
                );
                ai.outbox.actor = later_actor_effects;
            }
        }

        let still_pending = self
            .world
            .entities
            .get(owner)
            .and_then(Entity::ai_controller)
            .is_some_and(|ai| !ai.outbox.reentrant.owner_work.is_empty());
        assert!(
            !still_pending,
            "AI owner {} exceeded recursive FIFO bound {MAX_OWNER_WORK}",
            owner.index()
        );
    }

    #[cfg(test)]
    pub(crate) fn drain_ai_state_change_notifications_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: crate::element::EntityId,
    ) {
        self.drain_ai_owner_work_for(sim, assets, owner);
    }

    /// Initialize the engine for the campaign's current mission.
    ///
    /// The campaign must already be stored in `self.mission_domain.campaign`.
    /// Pulls the mission name, proto-level filename, and mission type
    /// from the campaign state, then delegates to `initialize_from_mission`.
    ///
    /// Called from `Engine::new` when `EngineArgs::level` is set.
    pub(crate) fn initialize_from_campaign(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &mut LevelAssets,
        staging: &mut LevelLoadStaging,
        loaded: crate::level_data::LoadedLevel,
        level_directory: &str,
        bg_pixel_dims: (f32, f32),
        progress: &mut dyn FnMut(f32),
    ) -> Result<(), EngineError> {
        let campaign = Some(&self.mission_domain.campaign)
            .expect("initialize_from_campaign: campaign not set on engine");
        let idx = campaign
            .current_mission_idx
            .expect("initialize_from_campaign: no current mission set");
        let profile = campaign.missions[idx].profile(&assets.profile_manager);
        let mission_filename = profile.mission_filename.clone();
        let proto_level_filename = profile.proto_level_filename.clone();

        self.initialize_from_mission(
            sim,
            assets,
            staging,
            &mission_filename,
            &proto_level_filename,
            loaded,
            level_directory,
            bg_pixel_dims,
            progress,
        )?;

        // `RHEngine::InitializeMiscFromProtoStream` owns `mbForestLevel`.
        // Keep the value loaded from the proto MISC chunk above rather than
        // deriving it from the campaign-map location.  Those are usually
        // aligned, but not identical: SherwoodOutro uses the Sherwood proto
        // while its profile location is Cross2.

        Ok(())
    }

    /// Sync the post-mission soldier counts into the campaign's running
    /// totals.  `LIVING_SOLDIERS_VALUE` and `DEAD_SOLDIERS_VALUE` are
    /// accumulated only at mission end.  Money and score are NOT
    /// synced here: they are credited continuously during gameplay
    /// through `EngineInner::add_campaign_value`'s side effects
    /// (the RANSOM/SCORE branches of `Campaign::add_value`), so
    /// re-adding them at mission end would double-count.
    pub fn sync_stats_to_campaign(&self, campaign: &mut Campaign) {
        campaign.add_value(
            CampaignValue::LivingSoldiers,
            self.mission_domain.mission_stat.living_soldier_count as i32,
        );
        campaign.add_value(
            CampaignValue::DeadSoldiers,
            self.mission_domain
                .mission_stat
                .total_soldier_count
                .saturating_sub(self.mission_domain.mission_stat.living_soldier_count)
                as i32,
        );
    }

    /// Get the current mission's static profile from the campaign.
    ///
    /// Returns `None` if no current mission is set in the campaign.
    pub fn current_mission_profile<'a>(
        &self,
        campaign: &'a Campaign,
        profiles: &'a crate::profiles::ProfileManager,
    ) -> Option<&'a MissionProfile> {
        campaign
            .current_mission_idx
            .and_then(|idx| campaign.missions.get(idx))
            .map(|m| m.profile(profiles))
    }

    /// Check whether this is a Sherwood (HQ) mission based on the campaign.
    pub fn is_sherwood_mission(
        &self,
        campaign: &Campaign,
        profiles: &crate::profiles::ProfileManager,
    ) -> bool {
        self.current_mission_profile(campaign, profiles)
            .is_some_and(|p| p.location == MissionLocation::Sherwood)
    }

    // ─── Script command processing ──────────────────────────────

    /// Process all deferred commands from script native calls.
    /// Called after each script tick (Hourglass / CheckVictoryCondition).
    pub(crate) fn apply_host_commands(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        commands: Vec<crate::natives::EngineCommand>,
    ) {
        use crate::natives::EngineCommand;

        for cmd in commands {
            match cmd {
                EngineCommand::ScrollCameraTo { x, y, speed } => {
                    // Store the raw script point in `camera_wanted` so
                    // resize/zoom can re-derive the slide target later,
                    // and the centered+clamped result in `camera_slide`.
                    let pos = crate::coordinates::MapPoint::new(x, y);
                    self.feedback.cutscene_camera.camera_wanted = pos;
                    self.feedback.cutscene_camera.camera_slide =
                        self.check_location_is_valid_for_camera(pos);
                    self.control.speed = speed;
                }
                EngineCommand::JumpCameraTo { x, y } => {
                    // Snap the view to the script point and invalidate
                    // background validity so the next frame redraws.
                    let pos = crate::coordinates::MapPoint::new(x, y);
                    self.feedback.cutscene_camera.view_position =
                        self.check_location_is_valid_for_camera(pos);
                    self.feedback.pending_side_effects.invalidate_background = true;
                }
                EngineCommand::SetZoomLevel { zoom } => {
                    // `SetZoomLevel` only assigns the desired zoom; the
                    // `mechanized_zoom` flag flips later when the
                    // zoom-update loop notices `desired != current`.
                    // Guard the flag so a no-op `SetZoomLevel` at the
                    // current zoom doesn't prematurely flip it.
                    self.feedback.cutscene_camera.desired_zoom_factor = zoom;
                    if zoom != self.feedback.cutscene_camera.zoom_factor {
                        self.feedback.cutscene_camera.mechanized_zoom = true;
                    }
                }
                EngineCommand::StartDialog { dialog_id } => {
                    tracing::debug!("StartDialog({dialog_id}): queued for game session");
                    self.feedback
                        .pending_side_effects
                        .pending_dialogues
                        .push(dialog_id);
                    self.orders
                        .messenger
                        .send(Message::new(MessageType::Simple(SimpleMessage::ResetInput)));
                }
                EngineCommand::DisplayMap { show } => {
                    self.feedback
                        .pending_side_effects
                        .pending_minimap_display_maps
                        .push((show, false));
                }
                EngineCommand::DisplayConsole => {
                    tracing::debug!("DisplayConsole: queued for UI system");
                    self.feedback.pending_side_effects.pending_show_console = true;
                    self.orders.messenger.send(Message::new(MessageType::Simple(
                        SimpleMessage::DisplayConsole,
                    )));
                }
                EngineCommand::CustomizeMinimapDisplay {
                    actor_handle,
                    dot_type,
                } => {
                    // Validate the dot code against the known
                    // CUSTOM_DOT_* whitelist, gate the `_MULTI` variants
                    // on `is_human()` (codes 111/222/333/444), and overwrite
                    // the PC / Villain / Civilian outline colour slots
                    // for the codes that select a class.
                    use crate::element_kinds::OutlineColorName;
                    use crate::element_kinds::outline_colors;
                    use crate::minimap::CustomDot;
                    let Some(id) = self.entity_id_for_actor_handle(actor_handle) else {
                        tracing::warn!(
                            "CustomizeMinimapDisplay: invalid actor handle {actor_handle}"
                        );
                        continue;
                    };
                    let Some(entity) = self.get_entity_mut(id) else {
                        tracing::warn!(
                            "CustomizeMinimapDisplay: invalid actor handle {actor_handle}"
                        );
                        continue;
                    };
                    // Match a fixed whitelist of CUSTOM_DOT_* values.
                    // Any other code → log + skip both the dot update
                    // and the outline-colour write.
                    let dot_val = dot_type as u16;
                    let dot = CustomDot::try_from_u16(dot_val);
                    let Some(dot) = dot else {
                        tracing::warn!(
                            "Script Error: Trying to customize minimap display with illegal dot ID ({:#x}).",
                            dot_val
                        );
                        continue;
                    };
                    // `_MULTI` codes require an is_human() target;
                    // log + early return otherwise.
                    if dot.requires_human() && !entity.is_human() {
                        tracing::warn!(
                            "Script Error: Minimap multi-state display codes are only valid for humans (got {dot_val})."
                        );
                        continue;
                    }
                    entity.element_data_mut().custom_minimap_dot = dot_val;
                    // Second switch — overwrite outline colour slots
                    // for PC / Villain / Civilian variants.  The
                    // `_DEAD` / `_LYING` / `_MULTI` variants also fall
                    // into these palette groups.
                    let palette = match dot {
                        CustomDot::Pc
                        | CustomDot::PcLying
                        | CustomDot::PcDead
                        | CustomDot::PcMulti => Some((
                            outline_colors::pc_default(),
                            outline_colors::pc_hidden(),
                            outline_colors::pc_target(),
                        )),
                        CustomDot::Villain
                        | CustomDot::VillainLying
                        | CustomDot::VillainDead
                        | CustomDot::VillainMulti => Some((
                            outline_colors::npc_evil_default(),
                            outline_colors::npc_evil_hidden(),
                            outline_colors::npc_evil_target(),
                        )),
                        CustomDot::Civilian
                        | CustomDot::CivilianLying
                        | CustomDot::CivilianDead
                        | CustomDot::CivilianMulti => Some((
                            outline_colors::npc_good_default(),
                            outline_colors::npc_good_hidden(),
                            outline_colors::npc_good_target(),
                        )),
                        _ => None,
                    };
                    if let Some((default, hidden, target)) = palette {
                        let colors = &mut entity.element_data_mut().outline_colors;
                        colors[OutlineColorName::Default as usize] = default;
                        colors[OutlineColorName::Hidden as usize] = hidden;
                        colors[OutlineColorName::Target as usize] = target;
                    }
                }
                EngineCommand::DefineFlatTrajectoryZone {
                    location_handle,
                    apex_height,
                } => {
                    // Resolve the location handle to the matching script
                    // zone index and transform its script sector into
                    // an apex sector.
                    //
                    // Script-location payload indices are laid out as
                    // `[script_points..., script_sectors...]`; the sector
                    // slice starts at `script_location_count - script_zone_data.len()`.
                    let points_count = assets
                        .scripts
                        .location_positions
                        .len()
                        .saturating_sub(self.script_domains.zones.scripts.len());
                    let Some(loc_idx) =
                        crate::natives::ScriptHandleCodec::location_index(location_handle)
                    else {
                        tracing::warn!(
                            "DefineFlatTrajectoryZone(loc={location_handle}): invalid location handle"
                        );
                        continue;
                    };
                    if loc_idx < points_count || loc_idx >= assets.scripts.location_positions.len()
                    {
                        tracing::warn!(
                            "DefineFlatTrajectoryZone(loc={location_handle}): handle is not a script zone sector"
                        );
                        continue;
                    } else {
                        let zone_idx = loc_idx - points_count;
                        if let Some(zone) = self.script_domains.zones.scripts.get_mut(zone_idx) {
                            if zone.script_associated {
                                tracing::warn!(
                                    "DefineFlatTrajectoryZone(loc={location_handle}): \
                                     cannot convert script-associated sector to apex"
                                );
                            } else {
                                zone.transform_into_apex(apex_height as f32);
                                // Flip the APEX flag on the corresponding
                                // grid sector so `is_apex()` queries see it.
                                // The flag lives on the runtime overlay (not
                                // the static sector_type) so the geometry
                                // arena stays purely level-loaded.
                                if let Some(&grid_idx) =
                                    assets.scripts.zone_grid_indices.get(zone_idx)
                                {
                                    self.world.fast_grid.or_sector_type_overlay(
                                        grid_idx,
                                        crate::sector::SectorType::APEX,
                                    );
                                }
                            }
                        } else {
                            tracing::warn!(
                                "DefineFlatTrajectoryZone(loc={location_handle}): zone {zone_idx} out of range"
                            );
                        }
                    }
                }
                EngineCommand::ChooseVictoryDefeatText { id } => {
                    self.mission_domain.state.victory_defeat_id = id as u32;
                }
                EngineCommand::DisplayPopupText { text_id } => {
                    tracing::debug!("DisplayPopupText({text_id}): queued for UI system");
                    self.feedback
                        .pending_side_effects
                        .pending_popup_texts
                        .push(text_id);
                    self.orders
                        .messenger
                        .send(Message::new(MessageType::Simple(SimpleMessage::ResetInput)));
                }
                EngineCommand::DisplaySherwoodReport => {
                    tracing::debug!("DisplaySherwoodReport: queued for UI system");
                    self.feedback.pending_side_effects.pending_sherwood_report = true;
                    self.orders
                        .messenger
                        .send(Message::new(MessageType::Simple(SimpleMessage::ResetInput)));
                }
                EngineCommand::FadeToBlack { speed } => {
                    // The original `FadeToBlack` runs `2 * speed`
                    // iterations of a per-pixel-scale ramp, each
                    // followed by a present.  No engine update happens
                    // between iterations, so the game is genuinely
                    // frozen for the duration of the fade.  We split
                    // that into:
                    //   - `pending_side_effects.fade_to_black`: per-pixel
                    //     ramp drained by the host renderer (alpha-blend
                    //     overlay matching `current_alpha`).
                    //   - `fade_freeze_frames_remaining`: presentation
                    //     countdown read before the hourglass wrapper
                    //     touches any game clock or timer. The trigger
                    //     tick presents frame one, leaving `2*speed - 1`
                    //     frozen presentation frames. This is the only
                    //     blocking native in the entire script API
                    //     (verified across all shipped `.scb` files;
                    //     called once total, in `H04_Lei_VL`
                    //     `ProcessMessage(11)`), so a per-engine freeze
                    //     countdown beats generic VM yield/resume infra.
                    let s = speed.max(0) as u32;
                    let total_frames = s.saturating_mul(2);
                    self.feedback.pending_side_effects.fade_to_black = Some(if s == 0 {
                        None
                    } else {
                        Some(crate::engine::types::FadeToBlack {
                            speed: s,
                            frames_remaining: total_frames,
                        })
                    });
                    self.set_fade_freeze_frames_remaining(total_frames.saturating_sub(1));
                }
                EngineCommand::SetOutlineDisplay { display: show } => {
                    // Forward `MSG_SWITCH_MASKED_DISPLAY` when the
                    // state actually changes.  The rendering side
                    // (`game_render.rs:814` et al.) already reads
                    // `host.input.draw_hidden` to switch entities into
                    // the masked/outline draw mode.
                    self.feedback.pending_side_effects.set_draw_hidden = Some(show);
                }
                EngineCommand::SetActorLocation {
                    actor_handle,
                    x,
                    y,
                    dest_layer_sector,
                    spawn_elevation_probe,
                } => {
                    // SetPositionMap → SetLayer/SetSector →
                    // SetObstacle(GetProjectionArea) → ComputePositionAll.
                    // The native already wrote `position_map` and
                    // (for static script destinations) `layer` /
                    // `sector`; here we refresh the position interface,
                    // the grid cell, and — when a new floor landed the
                    // actor on a different projection-area obstacle —
                    // re-bind obstacle/material too.
                    let Some(id) = self.entity_id_for_actor_handle(actor_handle) else {
                        tracing::warn!("SetActorLocation: invalid actor handle {actor_handle}");
                        continue;
                    };
                    let Some(is_actor) = self
                        .world
                        .entities
                        .get(id)
                        .map(|entity| entity.actor_data().is_some())
                    else {
                        tracing::warn!("SetActorLocation: actor {actor_handle} missing entity");
                        continue;
                    };
                    if let Some((layer, sector_num)) = dest_layer_sector {
                        let sector = crate::position_interface::SectorHandle::new(sector_num);
                        let entity = self
                            .world
                            .entities
                            .get_mut(id)
                            .expect("SetActorLocation actor vanished before layer mutation");
                        entity.element_data_mut().set_layer(layer);
                        entity.element_data_mut().set_sector(sector);
                    }
                    let pt = crate::coordinates::MapPoint { x, y };
                    if !is_actor {
                        // Non-actor entities don't need the full actor
                        // reproject dance; refresh the basic grid.
                        let entity = self
                            .world
                            .entities
                            .get_mut(id)
                            .expect("SetActorLocation entity vanished before grid refresh");
                        entity.element_data_mut().set_position_map(pt);
                        entity.element_data_mut().update_grid_cell();
                        continue;
                    }
                    let entity = self
                        .world
                        .entities
                        .get_mut(id)
                        .expect("SetActorLocation actor vanished before position refresh");
                    let pi = entity.position_iface_mut();
                    pi.set_map_position(pt);
                    let ed = entity.element_data_mut();
                    ed.set_position_map(pt);
                    ed.update_grid_cell();

                    // Motion-area validation: check the destination
                    // sector after the position/layer/sector writes
                    // but before obstacle refresh / display-order /
                    // spawn-elevation — on failure log
                    // `VERBOTEN SCRIPT : Character not lying on motion
                    // area (%f,%f) !` and return, leaving the partial
                    // state writes in place.  Required ordering: if
                    // the destination sector isn't a motion area,
                    // skip the rest.
                    if let Some((_layer, sector_num)) = dest_layer_sector {
                        let sector_handle = crate::sector::SectorNumber::new(sector_num as i16);
                        let valid = self
                            .grid_sector_by_number(sector_handle)
                            .map(|gs| gs.sector_type.is_motion() && gs.sector_type.is_area())
                            .unwrap_or(false);
                        if !valid {
                            tracing::warn!(
                                "VERBOTEN SCRIPT : Character not lying on motion area ({}, {}) !",
                                pt.x,
                                pt.y,
                            );
                            continue;
                        }
                    }

                    // ComputeDisplayOrder(NULL, true) — passing a null
                    // reference element zeroes any stale
                    // `display_order_ref` so a teleported actor that
                    // had been carried/attached doesn't keep its prior
                    // z-sort anchor.
                    let Some(entity) = self.world.entities.get_mut(id) else {
                        continue;
                    };
                    let sprite = entity.sprite_mut();
                    sprite.display_order_ref = None;
                    sprite.behind_display_order_ref = false;

                    // Projection-area refresh: if the native told us the
                    // destination's layer/sector, look up the new
                    // projection area and stamp its obstacle + material
                    // on the actor.  Computed (non-static) locations
                    // don't carry layer/sector so the refresh is
                    // skipped — the obstacle only gets rebound when
                    // the destination was a real script point or
                    // script sector.
                    if let Some((layer, sector_num)) = dest_layer_sector {
                        let new_obstacle =
                            self.get_projection_area_index(assets, sector_num, layer, pt);
                        let new_material = new_obstacle.and_then(|oi| {
                            self.sight_obstacles(assets).get(oi as usize).map(|obs| {
                                crate::element::GameMaterial::from_u32(obs.material as u32)
                            })
                        });
                        let new_obstacle_handle =
                            new_obstacle.and_then(crate::position_interface::ObstacleHandle::new);
                        let plane = crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                            new_obstacle_handle,
                            assets.static_sight_obstacles.as_slice(),
                        );
                        if let Some(entity) = self.world.entities.get_mut(id) {
                            let ed = entity.element_data_mut();
                            ed.set_obstacle_index(new_obstacle_handle, plane);
                            if let Some(mat) = new_material {
                                ed.set_material(mat);
                            }
                        }
                    }

                    // Spawn-elevation compose (RecordEnterGame path):
                    //     elevation = position_to_point_3d(destination).z;
                    //     origin.y = outside.y + elevation;
                    //     origin.z = elevation;
                    //     set_position(origin);
                    // When `spawn_elevation_probe` is `Some((dx, dy))` we
                    // evaluate the destination sector's top plane at the
                    // *inside* probe point and overwrite the actor's 3D
                    // position so the outside-of-map spawn sits at the
                    // same altitude as where it's about to walk to.  The
                    // earlier `set_position_map` call derived Z from the
                    // actor's stale cached plane — acceptable for
                    // ordinary SetActorLocation but wrong for an
                    // outside-of-map enter-game spawn.
                    if let (Some((layer, sector_num)), Some((probe_x, probe_y))) =
                        (dest_layer_sector, spawn_elevation_probe)
                    {
                        let handle = crate::position_interface::SectorHandle::new(sector_num);
                        let elev = self
                            .position_to_point_3d(assets, handle, layer, probe_x, probe_y)
                            .z;
                        if let Some(entity) = self.world.entities.get_mut(id) {
                            // `set_position` writes the 3D point and
                            // calls `recompute_from_3d`, which rederives
                            // `position_map` / sprite / move_box from
                            // the new `(x, y + elev, elev)` — preserving
                            // the iso invariant `map.y = position.y -
                            // position.z`.  The earlier
                            // `set_position_map(x, y)` above routed
                            // through the actor's stale cached plane at
                            // a 2D point that's outside the map; this
                            // pass corrects both Z and map-Y from the
                            // destination's projection-area top plane.
                            let pi = entity.position_iface_mut();
                            pi.set_position(crate::coordinates::WorldPoint3D {
                                x,
                                y: y + elev,
                                z: elev,
                            });
                            entity.element_data_mut().update_grid_cell();
                        }
                    }
                }
                EngineCommand::Win { show_window } => {
                    self.win(show_window);
                }
                EngineCommand::SetScrollStatus {
                    scroll_handle,
                    status,
                } => {
                    // Set scroll status: write status, run minimap-dot
                    // update, force animation `BonusThree` when entering
                    // Opened.  The native pre-validates handle/type/
                    // range, so the script handle is an actor handle
                    // for a scroll entity and `status` is in 0..=3.
                    let Some(eid) = self.entity_id_for_actor_handle(scroll_handle) else {
                        continue;
                    };
                    let st = ScrollStatus::from_i32(status);
                    self.set_scroll_status(eid, st);
                    if matches!(st, crate::engine::scroll_reveal::ScrollStatus::Opened)
                        && let Some(entity) = self.get_entity_mut(eid)
                        && let Some(obj) = entity.object_data_mut()
                    {
                        obj.animation = crate::order::OrderType::BonusThree;
                    }
                }
                EngineCommand::ScriptMakePCCrouched { actor_handle } => {
                    // Validate the handle is a PC, then delegate to
                    // `actor_make_crouched`, which either rewrites an
                    // in-flight movement sequence to its crouched
                    // variant or launches a brand-new
                    // `Command::CrouchDown` so the actor plays the
                    // crouch-down animation.
                    let Some(eid) = self.entity_id_for_actor_handle(actor_handle) else {
                        tracing::error!(
                            "Script Error: The Actor in MakePCCrouched is invalid (handle {actor_handle})"
                        );
                        continue;
                    };
                    if !matches!(self.get_entity(eid), Some(crate::element::Entity::Pc(_))) {
                        tracing::error!(
                            "Script Error: The Actor in MakePCCrouched is invalid (handle {actor_handle})"
                        );
                        continue;
                    }
                    self.actor_make_crouched(sim, eid);
                }
                EngineCommand::SetMobileActive {
                    mobile_index,
                    active,
                } => {
                    let mobile = self
                        .world
                        .mobile_elements
                        .get_mut(usize::from(mobile_index))
                        .unwrap_or_else(|| {
                            panic!("SetMobileActive references missing mobile {mobile_index}")
                        });
                    mobile.set_active(active);
                    let sprite_ids = mobile.sprite_ids.clone();
                    for sprite_id in sprite_ids {
                        let fx = self
                            .world
                            .entities
                            .get_mut(sprite_id)
                            .and_then(crate::element::Entity::as_fx_mut)
                            .unwrap_or_else(|| {
                                panic!(
                                    "mobile {mobile_index} child {sprite_id} is missing or non-FX"
                                )
                            });
                        fx.element.active = active;
                    }
                }
                EngineCommand::MarkPc { actor_handle } => {
                    // Resolve the script handle to an EntityId and route
                    // it to the host via pending_side_effects.  The sim
                    // can't draw, so it hands the ID off to the host's
                    // outline pass, which flashes the outline for one
                    // frame.
                    if let Some(eid) = self.entity_id_for_actor_handle(actor_handle) {
                        if matches!(self.get_entity(eid), Some(crate::element::Entity::Pc(_))) {
                            self.feedback
                                .pending_side_effects
                                .pending_mark_pc_ids
                                .push(eid);
                        } else {
                            tracing::warn!(
                                "MarkPc: handle {actor_handle} does not resolve to a PC"
                            );
                        }
                    }
                }
                EngineCommand::UpdateInformationBars => {
                    // The original `UpdateInformationBars` does two
                    // things:
                    //   (a) tears down and rebuilds the blazon bar
                    //       vs. the mission-requirements widget based
                    //       on `ProduceBlazons()` and the next-mission
                    //       profile type.
                    //   (b) calls `UpdateBlazonStatus()` on the blazon
                    //       bar so its counter matches the current
                    //       human-status / mission-stat values.
                    //
                    // Our HUD (see `game_render.rs`, `hud_text.rs`,
                    // `ui_panel.rs`) is immediate-mode: every frame
                    // re-reads mission + campaign + money state
                    // directly from the engine, campaign, and
                    // mission-stat it already has in scope.  There are
                    // no cached widget instances to recreate, and
                    // money / blazon counters do not cache their
                    // displayed value.  Therefore (b) is a no-op —
                    // the next frame will render the updated counters
                    // automatically.
                    //
                    // For (a), the blazon-bar and mission-requirements
                    // widgets are data-computation modules (see
                    // `widget/blazon_bar.rs`, `widget/requirements.rs`)
                    // that the immediate-mode HUD reads per-frame.
                    // Nothing to cache on the engine side: derive the
                    // states here so the log/trace reflects what the
                    // next HUD frame will show.
                    if let Some(campaign) = Some(&self.mission_domain.campaign) {
                        // `Game::is_men_to_blazon_conversion` is reflected in
                        // the engine-owned mission UI domain by the
                        // `SetMenToBlazonConversionMode` player command.
                        // Read that state here so the blazon bar can
                        // switch to next-mission targeting during
                        // conversion mode without needing a `&Game`
                        // borrow at the engine tick.
                        let men_to_blazon =
                            self.script_domains.mission_ui.men_to_blazon_conversion_mode;
                        let blinking = self
                            .script_domains
                            .mission_ui
                            .active_blinking_blazons(self.control.frame_counter);
                        let bb = crate::widget_state::blazon_bar::build_blazon_bar_state(
                            campaign,
                            &assets.profile_manager,
                            men_to_blazon,
                            blinking,
                        );
                        let mission_team: Vec<crate::profiles::CharacterProfileIdx> =
                            campaign.mission_team_profile_indices();
                        let selected: Vec<crate::profiles::CharacterProfileIdx> =
                            self.players.seats[0]
                                .selection
                                .iter()
                                .filter_map(|&id| match self.get_entity(id)? {
                                    crate::element::Entity::Pc(pc) => Some(pc.pc.profile_index),
                                    _ => None,
                                })
                                .collect();
                        let req = campaign.next_mission_idx.and_then(|idx| {
                            crate::widget_state::requirements::build_requirements_state(
                                campaign,
                                &assets.profile_manager,
                                idx,
                                &mission_team,
                                &selected,
                            )
                        });
                        tracing::debug!(
                            ?bb,
                            req_slots = req.as_ref().map(|r| r.slots.len()),
                            "UpdateInformationBars: recomputed HUD states"
                        );
                    } else {
                        tracing::debug!("UpdateInformationBars: no campaign — HUD states skipped");
                    }
                }
                EngineCommand::HeroSpeak { pc_id, expression } => {
                    self.hero_speaking(assets, pc_id, expression);
                }
                EngineCommand::MakeNoise {
                    noise_type,
                    x,
                    y,
                    layer,
                    sector,
                } => {
                    // Delegate to the shared broadcast path so scripted
                    // noises get the same AI dispatch and debug overlay
                    // as gameplay-triggered broadcasts.
                    use crate::parameters_ai;
                    let volume = match noise_type {
                        crate::ai::NoiseType::Logs => parameters_ai::NOISE_VOLUME_LOGS,
                        crate::ai::NoiseType::Drawbridge => parameters_ai::NOISE_VOLUME_DRAWBRIDGE,
                        // Unexpected — the native arm already rejects
                        // anything other than LOGS/DRAWBRIDGE.  Keep a
                        // sensible floor so a future arm extension
                        // doesn't silently broadcast zero-volume noise.
                        _ => parameters_ai::NOISE_VOLUME_PLOUF,
                    } as u16;
                    // `RHElementActorNPC::Noise(type, position)` resolves the
                    // script point through `PositionToPoint3D` before the
                    // hearing test.  Preserve the location sector so noises
                    // on roofs and other raised projection areas originate at
                    // their actual elevation.
                    let source = self.position_to_point_3d(
                        assets,
                        crate::position_interface::SectorHandle::new(sector),
                        layer,
                        x,
                        y,
                    );
                    tracing::debug!(
                        noise_type = ?noise_type,
                        x,
                        y,
                        layer,
                        sector,
                        elevation = source.z,
                        "dispatching scripted noise"
                    );
                    self.broadcast_noise_synchronously(
                        sim,
                        assets,
                        noise_type,
                        crate::coordinates::MapPoint::new(x, y),
                        layer,
                        volume,
                        source.z as u16,
                        None,
                    );
                }
            }
        }
    }

    /// Apply the positioning side of `PutActorInBuilding`:
    /// SetActive(false), mark hidden-in-building, move to the
    /// building's special layer + sector, teleport onto the first gate's
    /// `point_in`, and DisableAllActionsTemp for PCs.
    fn put_actor_in_building(&mut self, actor: i32, building: i32) {
        let Some(actor_id) = self.entity_id_for_actor_handle(actor) else {
            tracing::warn!("PutActorInBuilding: invalid actor handle {actor}");
            return;
        };
        let Some(bld_idx) = crate::natives::ScriptHandleCodec::building_index(building) else {
            tracing::warn!("PutActorInBuilding: invalid building handle {building}");
            return;
        };

        // Look up the first gate's `point_in` and the building's sector
        // number. Sector number comes from the grid sector tagged
        // `building_index == bld_idx` (populated at level load).
        let (gate_point_in, sector_num) = {
            let gate_handle = self
                .script_domains
                .buildings
                .gates
                .get(bld_idx)
                .and_then(|g| g.first())
                .copied();
            let point_in = gate_handle
                .and_then(crate::natives::ScriptHandleCodec::door_index)
                .and_then(|di| self.script_domains.interactables.doors.get(di))
                .map(|d| d.point_in);
            let sn = self.world.fast_grid.level.sectors.iter().find_map(|gs| {
                if gs.building_index == crate::sector::BuildingIdx::new(bld_idx as u16) {
                    Some(gs.sector_number)
                } else {
                    None
                }
            });
            (point_in, sn)
        };

        let Some(point_in) = gate_point_in else {
            tracing::warn!(
                "PutActorInBuilding: building {building} has no gates — cannot position actor"
            );
            return;
        };
        let Some(sector_num) = sector_num else {
            tracing::warn!(
                "PutActorInBuilding: building {building} has no grid sector — cannot position actor"
            );
            return;
        };

        let special_layer = self.world.fast_grid.level.special_layer;

        let is_pc;
        let carried_handle: Option<i32>;
        if let Some(entity) = self.world.entities.get_mut(actor_id) {
            let elem = entity.element_data_mut();
            elem.hidden_in_building = true;
            elem.active = false;
            elem.set_layer(special_layer);
            elem.set_sector(crate::position_interface::SectorHandle::new(u16::from(
                sector_num,
            )));
            elem.set_position_map(point_in);
            elem.update_grid_cell();
            // After `SetPositionMap` on the gate's point-in, re-derive
            // the sprite-space and 3D positions from the new map
            // position so the renderer / display-order pipeline picks
            // up the teleport on the first post-script frame instead
            // of mis-framing.
            if entity.actor_data().is_some() {
                let pi = entity.position_iface_mut();
                pi.set_map_position(point_in);
            }
            is_pc = entity.pc_data().is_some();
            carried_handle = entity
                .pc_data()
                .and_then(|pc| pc.carried)
                .map(crate::natives::ScriptHandleCodec::actor_handle);
            if is_pc && let Some(pc) = entity.pc_data_mut() {
                // DisableAllActionsTemp gates the
                // disabled_actions_temp loop on `playable` so a
                // non-playable PC kept inside the building doesn't
                // accumulate stale temp-disable flags.
                pc.disable_all_actions_temp();
            }
        } else {
            tracing::warn!("PutActorInBuilding: entity {actor:?} missing");
            return;
        }

        if is_pc {
            // Forward MSG_DISABLE_ALL_ACTIONS — counterpart to
            // DisableAllActionsTemp.
            self.orders.messenger.send(Message::pc(
                crate::messenger::PcMessage::DisableAllActionsTemp,
                None,
            ));

            // When the entering actor is a PC,
            // (a) recursively enter its carried actor, and
            // (b) re-enable existing occupants who are dead/unconscious
            //     and not being carried — their corpses should render
            //     inside the building.
            if let Some(carried_h) = carried_handle
                && carried_h != 0
            {
                if let Some(carried_id) = self.entity_id_for_actor_handle(carried_h)
                    && let Some(carried_entity) = self.world.entities.get_mut(carried_id)
                {
                    let elem = carried_entity.element_data_mut();
                    elem.hidden_in_building = true;
                    elem.active = false;
                    elem.set_layer(special_layer);
                    elem.set_sector(crate::position_interface::SectorHandle::new(u16::from(
                        sector_num,
                    )));
                    elem.set_position_map(point_in);
                    elem.update_grid_cell();
                    if carried_entity.actor_data().is_some() {
                        let pi = carried_entity.position_iface_mut();
                        pi.set_map_position(point_in);
                    }
                }
                // Push the carried into the occupants list.
                if bld_idx >= self.script_domains.buildings.occupants.len() {
                    self.script_domains
                        .buildings
                        .occupants
                        .resize(bld_idx + 1, Vec::new());
                }
                self.script_domains.buildings.occupants[bld_idx].push(carried_h);
                self.script_domains
                    .buildings
                    .actor_building
                    .insert(carried_h, building);
            }

            // Re-enable corpses already inside the building: walk the
            // occupants list and SetActive(true) on humans that are
            // (is_dead || unconscious) && carrier.is_none().
            let occupants: Vec<i32> = self
                .script_domains
                .buildings
                .occupants
                .get(bld_idx)
                .cloned()
                .unwrap_or_default();
            for occ_h in occupants {
                let Some(occ_id) = self.entity_id_for_actor_handle(occ_h) else {
                    continue;
                };
                let Some(occ) = self.world.entities.get_mut(occ_id) else {
                    continue;
                };
                let Some(hd) = occ.human_data() else { continue };
                let is_dead_or_ko = occ.is_dead() || hd.unconscious;
                let has_carrier = hd.carrier.is_some();
                if is_dead_or_ko && !has_carrier {
                    let elem = occ.element_data_mut();
                    elem.hidden_in_building = false;
                    elem.active = true;
                }
            }
        }

        tracing::debug!(
            "PutActorInBuilding: actor={actor} building={building} \
             → layer={special_layer}, sector={sector_num}, pos=({:.1},{:.1})",
            point_in.x,
            point_in.y,
        );
    }
}

#[cfg(test)]
mod script_context_tests {
    use super::*;
    use crate::scb::{ClassEntry, SCB_VERSION, ScbFile};

    fn empty_mission_script() -> MissionScript {
        let startup = ClassEntry {
            source_file: "script_context_test.scs".into(),
            class_name: "StartUp".into(),
            size_of_member_variables: 0,
            member_variables: Vec::new(),
            functions: Vec::new(),
            quads: Vec::new(),
        };
        MissionScript::from_scb(ScbFile {
            version: SCB_VERSION,
            classes: vec![startup],
        })
        .expect("minimal StartUp script must load")
    }

    #[test]
    fn mission_script_snapshot_round_trips_state_and_reattaches_program() {
        let mut script = empty_mission_script();
        script.state.globals.insert(7, 91);
        script
            .state
            .computed_locations
            .push(Some(crate::natives::ComputedScriptLocation {
                position: (12.5, -8.0),
                layer: Some(2),
                sector: Some(44),
                active: true,
                legacy_dummy: false,
            }));
        script.state.sequence_recorder.sequence_id = 3;
        script.state.sequence_recorder.recording = Some(crate::sequence::RecordingSession::new());
        let location_positions = std::sync::Arc::new(vec![(12.0, 34.0)]);
        script.attach_bindings(crate::natives::AttachedScriptBindings {
            script_location_count: 1,
            location_positions: location_positions.clone(),
            ..Default::default()
        });

        let hash_before = robin_util::state_hash::compute(&script);
        let program = script.manager.program.clone();
        let json = serde_json::to_string(&script).expect("serialize MissionScript");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse snapshot JSON");
        assert!(value.get("snapshot_version").is_none());
        let effect_keys = value["script_effects"]
            .as_object()
            .expect("ScriptEffects snapshot object")
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(effect_keys, ["ordered"].into_iter().collect());
        assert!(value.get("bindings").is_none());

        let mut decoded: MissionScript =
            serde_json::from_str(&json).expect("deserialize MissionScript");
        assert_eq!(decoded.bindings.script_location_count, 0);
        decoded.attach_program(program);
        decoded.attach_bindings(crate::natives::AttachedScriptBindings {
            script_location_count: 1,
            location_positions: location_positions.clone(),
            ..Default::default()
        });
        assert!(std::sync::Arc::ptr_eq(
            &decoded.bindings.location_positions,
            &location_positions
        ));
        assert_eq!(decoded.state.globals.get(&7), Some(&91));
        assert_eq!(decoded.state.computed_locations.len(), 1);
        assert!(decoded.state.sequence_recorder.recording.is_some());
        assert_eq!(robin_util::state_hash::compute(&decoded), hash_before);
    }

    #[test]
    fn customize_minimap_accepts_vip_dots_and_gates_vip_multi_to_humans() {
        let sim = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        engine
            .world
            .entities
            .push(Some(crate::element::Entity::Soldier(
                crate::element::ActorSoldier {
                    element: crate::element::ElementData {
                        kind: crate::element::ElementKind::ActorSoldier,
                        ..Default::default()
                    },
                    actor: crate::element::ActorData::default(),
                    human: crate::element::HumanData::default(),
                    npc: crate::element::NpcData::default(),
                    soldier: crate::element::SoldierData::default(),
                },
            )));
        engine.world.entities.push(Some(crate::element::Entity::Fx(
            crate::element::ElementFx {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::Fx,
                    custom_minimap_dot: crate::minimap::CustomDot::NotCustomized as u16,
                    ..Default::default()
                },
                fx: crate::element::FxData::default(),
            },
        )));
        let human_handle = crate::natives::ScriptHandleCodec::actor_handle_from_index(0);
        let non_human_handle = crate::natives::ScriptHandleCodec::actor_handle_from_index(1);

        engine.apply_host_commands(
            &sim,
            &LevelAssets::default(),
            vec![crate::natives::EngineCommand::CustomizeMinimapDisplay {
                actor_handle: human_handle,
                dot_type: crate::minimap::CustomDot::VipMulti as i32,
            }],
        );
        assert_eq!(
            engine
                .get_entity(engine.entity_id_for_index(0).expect("human entity"))
                .expect("human entity")
                .element_data()
                .custom_minimap_dot,
            crate::minimap::CustomDot::VipMulti as u16
        );

        engine.apply_host_commands(
            &sim,
            &LevelAssets::default(),
            vec![crate::natives::EngineCommand::CustomizeMinimapDisplay {
                actor_handle: non_human_handle,
                dot_type: crate::minimap::CustomDot::VipMulti as i32,
            }],
        );
        assert_eq!(
            engine
                .get_entity(engine.entity_id_for_index(1).expect("non-human entity"))
                .expect("non-human entity")
                .element_data()
                .custom_minimap_dot,
            crate::minimap::CustomDot::NotCustomized as u16
        );
    }

    #[test]
    fn patch_background_effects_invalidate_canonical_side_effects_immediately() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = EngineInner::new();
        engine.scripts.mission = Some(empty_mission_script());
        engine
            .script_domains
            .interactables
            .patches
            .push(crate::patch::Patch {
                integrate_in_background: true,
                ..Default::default()
            });
        let patch_index = crate::patch::PatchIndex::new(0).expect("zero is a valid patch index");

        engine.process_patch_effects(
            sim,
            &LevelAssets::default(),
            patch_index,
            vec![crate::patch::PatchEffect::SwapBackground { applied: true }],
        );
        assert!(engine.feedback.pending_side_effects.invalidate_background);

        engine.feedback.pending_side_effects.invalidate_background = false;
        engine.process_patch_effects(
            sim,
            &LevelAssets::default(),
            patch_index,
            vec![crate::patch::PatchEffect::RestoreBackground],
        );
        assert!(engine.feedback.pending_side_effects.invalidate_background);
    }

    #[test]
    fn external_this_actor_success_keeps_canonical_entity_ownership() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = crate::campaign::Campaign::default();
        engine.world.entities.push(None);
        engine.scripts.mission = Some(empty_mission_script());
        engine.attach_script_bindings(&LevelAssets::new());

        let result = engine.call_external_native_with_this(
            sim,
            &LevelAssets::new(),
            "ThisActor",
            &[],
            Some(99),
        );

        assert_eq!(result, Ok(99));
        assert_eq!(engine.world.entities.len(), 1);
        let script = engine
            .scripts
            .mission
            .as_ref()
            .expect("script remains installed");
        assert_eq!(script.active_call_frame_count(), 0);
    }

    #[test]
    fn native_mutation_writes_the_canonical_script_domains_in_place() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::interp::{HostFunctions, NativeStack};
        use crate::natives::NativeFn;

        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = crate::campaign::Campaign::default();
        engine.scripts.mission = Some(empty_mission_script());
        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                locked_pc: true,
                ..Default::default()
            });
        let assets = LevelAssets::new();
        engine.attach_script_bindings(&assets);
        let canonical_domains = std::ptr::addr_of_mut!(engine.script_domains);
        let canonical_entities = std::ptr::from_ref(&engine.world.entities);
        let door = crate::natives::ScriptHandleCodec::door_handle_from_index(0);

        let result =
            engine.with_script_session(sim, &assets, |script, script_domains, capabilities| {
                assert_eq!(
                    std::ptr::from_mut(script_domains),
                    canonical_domains,
                    "the native capability must borrow EngineInner's allocation"
                );
                assert_eq!(
                    capabilities.entities_owner_ptr(),
                    canonical_entities,
                    "the entity capability must borrow EngineInner's canonical allocation"
                );
                let mut stack = NativeStack::default();
                stack.push_i32(door);
                stack.push_i32(0);
                let mut context = crate::natives::NativeContext::with_bindings(
                    &mut script.script_effects,
                    &mut script.state,
                    script_domains,
                    &script.bindings,
                    capabilities,
                );
                HostFunctions::call(&mut context, NativeFn::SetDoorLockedPC as u32, &mut stack)
                    .expect_return("SetDoorLockedPC is synchronous")
            });

        assert_eq!(result, Some(0));
        assert!(!engine.script_domains.interactables.doors[0].locked_pc);
    }

    #[test]
    fn native_ai_mutation_writes_engine_inner_directly() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::interp::{HostFunctions, NativeStack};
        use crate::natives::NativeFn;

        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = crate::campaign::Campaign::default();
        engine.scripts.mission = Some(empty_mission_script());
        engine.ai.global.next_repulsive_point_id = 9;
        engine
            .ai
            .global
            .repulsive_points
            .push(crate::ai::RepulsivePoint::new(
                8,
                crate::ai::Position::default(),
                10.0,
                20.0,
                0,
            ));
        let assets = LevelAssets::new();
        engine.attach_script_bindings(&assets);
        let canonical_ai_global = std::ptr::addr_of_mut!(engine.ai.global);

        let result = engine.with_script_session(sim, &assets, |script, script_domains, queries| {
            let mut context = crate::natives::NativeContext::with_bindings(
                &mut script.script_effects,
                &mut script.state,
                script_domains,
                &script.bindings,
                queries,
            );
            assert_eq!(
                std::ptr::from_mut(context.ai_global_mut()),
                canonical_ai_global,
                "the native capability must borrow EngineInner's AI allocation"
            );
            let mut stack = NativeStack::default();
            stack.push_i32(8);
            HostFunctions::call(
                &mut context,
                NativeFn::DeleteRepulsivePoint as u32,
                &mut stack,
            )
            .expect_return("DeleteRepulsivePoint is synchronous")
        });

        assert_eq!(result, Some(0));
        assert!(engine.ai.global.repulsive_points.is_empty());
        assert_eq!(engine.ai.global.next_repulsive_point_id, 9);
    }

    #[test]
    #[should_panic(expected = "native dispatch requires live level attachments")]
    fn external_native_rejects_a_detached_live_script() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = EngineInner::new();
        engine.scripts.mission = Some(empty_mission_script());

        let _ = engine.call_external_native_with_this(
            sim,
            &LevelAssets::new(),
            "ThisActor",
            &[],
            Some(99),
        );
    }

    #[test]
    fn script_session_normal_return_restores_state_and_hash() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = crate::campaign::Campaign::default();
        engine.world.entities.push(None);
        engine.scripts.mission = Some(empty_mission_script());
        let assets = LevelAssets::new();
        engine.attach_script_bindings(&assets);
        let hash_before = robin_util::state_hash::compute(&engine);
        let canonical_entities = std::ptr::from_ref(&engine.world.entities);

        let result = engine.with_script_session(sim, &assets, |_script, _, capabilities| {
            assert_eq!(capabilities.entities_owner_ptr(), canonical_entities);
            73
        });

        assert_eq!(result, Some(73));
        assert_eq!(engine.world.entities.len(), 1);
        let script = engine.scripts.mission.as_ref().unwrap();
        assert_eq!(script.active_call_frame_count(), 0);
        assert_eq!(robin_util::state_hash::compute(&engine), hash_before);
    }

    #[test]
    fn script_callback_error_keeps_canonical_owners_in_place() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = crate::campaign::Campaign::default();
        engine.world.entities.push(None);
        engine.scripts.mission = Some(empty_mission_script());
        let assets = LevelAssets::new();
        engine.attach_script_bindings(&assets);

        let result: Result<(), &'static str> = engine
            .with_script_session(sim, &assets, |_script, _, _capabilities| {
                Err("simulated script error")
            })
            .unwrap();

        assert_eq!(result, Err("simulated script error"));
        assert_eq!(engine.world.entities.len(), 1);
        let script = engine.scripts.mission.as_ref().unwrap();
        assert_eq!(script.active_call_frame_count(), 0);
    }

    #[test]
    #[should_panic(expected = "simulated script panic")]
    fn script_callback_unwind_keeps_canonical_owners_in_place() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        struct VerifyRestoredOnUnwind(*const EngineInner);

        impl Drop for VerifyRestoredOnUnwind {
            fn drop(&mut self) {
                // SAFETY: the pointer targets the engine local below, which
                // outlives this verifier. All callback capability borrows have
                // ended before unwinding reaches this Drop implementation.
                let engine = unsafe { &*self.0 };
                assert_eq!(engine.world.entities.len(), 1);
                let script = engine.scripts.mission.as_ref().unwrap();
                assert_eq!(script.active_call_frame_count(), 0);
                assert!(
                    engine.script_domains.mission_ui.outline_display,
                    "canonical domain mutation survives callback unwind"
                );
                assert!(
                    engine.ai.global.golden_eye_mode,
                    "canonical AI-global mutation survives callback unwind"
                );
            }
        }

        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = crate::campaign::Campaign::default();
        engine.world.entities.push(None);
        engine.scripts.mission = Some(empty_mission_script());
        let assets = LevelAssets::new();
        engine.attach_script_bindings(&assets);
        let _verify = VerifyRestoredOnUnwind(&engine);

        let _ = engine.with_script_session(sim, &assets, |script, script_domains, capabilities| {
            script_domains.mission_ui.outline_display = true;
            {
                let mut context = crate::natives::NativeContext::with_bindings(
                    &mut script.script_effects,
                    &mut script.state,
                    script_domains,
                    &script.bindings,
                    capabilities,
                );
                context.ai_global_mut().golden_eye_mode = true;
            }
            panic!("simulated script panic");
        });
    }

    #[test]
    fn external_native_early_returns_without_touching_callback_state() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let mut engine = EngineInner::new();
        engine.world.entities.push(None);
        engine.scripts.mission = Some(empty_mission_script());

        let result = engine.call_external_native_with_this(
            sim,
            &LevelAssets::new(),
            "NotAnOriginalNative",
            &[],
            Some(99),
        );

        assert_eq!(result, Err("unknown native: NotAnOriginalNative".into()));
        assert_eq!(engine.world.entities.len(), 1);
        let script = engine
            .scripts
            .mission
            .as_ref()
            .expect("script remains installed");
        assert_eq!(script.active_call_frame_count(), 0);
    }
}

/// Schedule a finish for a freshly-activated source if its kind is
/// `Single` or `Volatile` — the two kinds that terminate on their own.
/// `Looped` never ends; `Delayed` runs its own sim-side re-roll in
/// `perform_hourglass` and isn't scheduled here.
///
/// A missing duration means the original cache lookup would return a
/// zero-length sample and complete it in the sound hourglass. Schedule
/// that same zero-length result and warn rather than inventing a duration.
fn schedule_source_finish(
    kind: &crate::sound_source::SoundSourceKind,
    sample_id: u32,
    source_index: usize,
    cur_frame: u32,
    durations: &super::SourceDurations,
    playing_sources: &mut Vec<crate::sound::PlayingSource>,
) {
    use crate::sound_source::SoundSourceKind;
    match kind {
        SoundSourceKind::Single | SoundSourceKind::Volatile => {
            let duration = durations.get(&sample_id).copied().unwrap_or_else(|| {
                tracing::warn!(
                    sample_id,
                    "sound source missing from source_durations table; \
                     scheduling zero-length completion"
                );
                0
            });
            playing_sources.push(crate::sound::PlayingSource {
                source_index: source_index as u32,
                finish_frame: cur_frame + duration,
            });
        }
        SoundSourceKind::Looped | SoundSourceKind::Delayed => {}
    }
}

#[cfg(test)]
mod sound_completion_tests {
    use super::*;
    use crate::sound::PlayingSource;
    use crate::sound_source::SoundSourceKind;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    #[test]
    fn source_finish_uses_exact_metadata_duration() {
        let durations = Arc::new(BTreeMap::from([(0x1234, 9)]));
        let mut playing = Vec::<PlayingSource>::new();

        schedule_source_finish(
            &SoundSourceKind::Single,
            0x1234,
            4,
            100,
            &durations,
            &mut playing,
        );

        assert_eq!(playing.len(), 1);
        assert_eq!(playing[0].source_index, 4);
        assert_eq!(playing[0].finish_frame, 109);
    }

    #[test]
    fn missing_source_duration_schedules_zero_length_completion() {
        let durations = Arc::new(BTreeMap::new());
        let mut playing = Vec::<PlayingSource>::new();

        schedule_source_finish(
            &SoundSourceKind::Volatile,
            0x5678,
            7,
            100,
            &durations,
            &mut playing,
        );

        assert_eq!(playing.len(), 1);
        assert_eq!(playing[0].source_index, 7);
        assert_eq!(
            playing[0].finish_frame, 100,
            "missing samples complete at the next drain, never after a fabricated 75 frames"
        );
    }
}

/// Walk every active source in `sound_sim.sources` and schedule a
/// fresh finish for the `Single` / `Volatile` ones.  Called from the
/// `ResumeAll` dispatch so a script-triggered suspend/resume
/// round-trip produces the same kind-specific termination the host
/// used to drive via audio-backend playback completion.
fn schedule_source_finishes_for_all_active(
    sound_sim: &mut crate::sound::SoundSimState,
    durations: &super::SourceDurations,
    cur_frame: u32,
) {
    for i in 0..sound_sim.sources.num_sources() {
        let Some(src) = sound_sim.sources.get(i) else {
            continue;
        };
        if !src.active {
            continue;
        }
        let kind = src.source_kind;
        let id = src.id;
        // Re-arming duplicates would stack a second finish on top of
        // any existing entry, so cancel first.
        sound_sim
            .playing_sources
            .retain(|p| p.source_index as usize != i);
        schedule_source_finish(
            &kind,
            id,
            i,
            cur_frame,
            durations,
            &mut sound_sim.playing_sources,
        );
    }
}

impl EngineInner {
    /// Dispatch a single native function from outside the script VM
    /// (HTTP-RPC, debug console, etc.).
    ///
    /// Goes through the same disjoint-owner boundary script callbacks use,
    /// so any side-effect commands the
    /// native queues (camera, dialog, sequence Start/Thanx, sound,
    /// deferred game-logic) are drained as if a script had made the
    /// call.
    ///
    /// `args` are pushed onto a fresh `NativeStack` in script-source
    /// order (i.e. `args[0]` is the first argument to the native, and
    /// will be popped *last* — matches the `Param`/`Pop` LIFO contract).
    ///
    /// When `this_actor` is `Some`, the standalone frame binds `ThisActor`
    /// for the duration of the call. Pass `None` for a receiver-free frame.
    pub fn call_external_native(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        native_name: &str,
        args: &[i32],
    ) -> Result<i32, String> {
        self.call_external_native_with_this(sim, assets, native_name, args, None)
    }

    /// Like [`Self::call_external_native`], but with an explicit
    /// `ThisActor` receiver installed in the transient call frame.
    pub fn call_external_native_with_this(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        native_name: &str,
        args: &[i32],
        this_actor: Option<i32>,
    ) -> Result<i32, String> {
        use crate::interp::NativeStack;
        use crate::natives::NativeFn;

        // Resolve name -> index. The enum implements `IntoStaticStr`
        // (one-way), so reverse lookup is a small linear scan over the
        // ~291 known indices. Comparison is case-insensitive — script
        // source uses CamelCase but JSON callers may not match exactly.
        let mut found_index: Option<u32> = None;
        for i in 0u32..512 {
            if let Ok(n) = NativeFn::try_from(i) {
                let s: &'static str = n.into();
                if s.eq_ignore_ascii_case(native_name) {
                    found_index = Some(i);
                    break;
                }
            }
        }
        let Some(index) = found_index else {
            return Err(format!("unknown native: {native_name}"));
        };

        if self.scripts.mission.is_none() {
            return Err("no mission script loaded (no mission running)".into());
        }

        let base_frame = this_actor.map_or_else(
            crate::natives::ScriptCallFrame::default,
            crate::natives::ScriptCallFrame::actor,
        );
        self.scripts
            .mission
            .as_mut()
            .expect("mission-script presence checked above")
            .push_active_driver_frame(base_frame);
        let mut active = vec![ActiveScriptCall {
            target: this_actor.map_or(ScriptVmKey::Global, ScriptVmKey::Actor),
            frame: base_frame,
            counts_toward_depth: false,
        }];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let outcome = self
                .with_script_session_in_driver(
                    sim,
                    assets,
                    |script, script_domains, capabilities| {
                        let mut stack = NativeStack::default();
                        for &a in args {
                            stack.push_i32(a);
                        }
                        let mut native_context = crate::natives::NativeContext::with_call_frame(
                            &mut script.script_effects,
                            &mut script.state,
                            script_domains,
                            &script.bindings,
                            capabilities,
                            base_frame,
                        );
                        crate::interp::HostFunctions::call(&mut native_context, index, &mut stack)
                    },
                )
                .expect("mission-script presence checked above");
            self.drain_script_effects_with_active(sim, assets, &mut active)?;
            match outcome {
                crate::interp::NativeCallOutcome::Return(value) => Ok(value),
                crate::interp::NativeCallOutcome::Yield(request) => {
                    let operation_result = match request.operation {
                        crate::interp::NativeOperation::ScriptCall(call) => {
                            let frame = match call.script_this {
                                crate::interp::NestedCallScriptThis::TargetActor => {
                                    base_frame.with_script_this(call.actor_handle)
                                }
                                crate::interp::NestedCallScriptThis::PreserveCaller => base_frame,
                            };
                            self.call_script_vm_inner(
                                sim,
                                assets,
                                ScriptVmKey::Actor(call.actor_handle),
                                &call.fn_name,
                                &call.params,
                                frame,
                                &mut active,
                            )?
                        }
                        crate::interp::NativeOperation::SequenceAction(operation) => {
                            self.drive_detached_sequence_operation(
                                sim,
                                assets,
                                operation,
                                &mut active,
                            )?;
                            0
                        }
                        crate::interp::NativeOperation::EngineAction(action) => self
                            .execute_synchronous_script_request(sim, assets, action, &mut active)?,
                    };
                    Ok(match request.resume {
                        crate::interp::ResumePolicy::OperationResult => operation_result,
                        crate::interp::ResumePolicy::Fixed(value) => value,
                    })
                }
            }
        }));
        let popped = active.pop();
        debug_assert!(popped.is_some_and(|call| !call.counts_toward_depth));
        self.scripts
            .mission
            .as_mut()
            .expect("mission script vanished while restoring external-native guard")
            .pop_active_driver_frame(base_frame);
        match result {
            Ok(result) => result.map_err(|error: ScriptDriverError| error.detail),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}
