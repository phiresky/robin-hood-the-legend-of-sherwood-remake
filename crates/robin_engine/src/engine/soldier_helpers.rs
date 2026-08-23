//! Soldier-specific helpers used by the engine, scripts, and cheats.
//!
//! Small soldier-actor methods that don't fit naturally into any of
//! the larger modules — attentive-mode requests, drunken-step
//! perturbation, etc.

use super::movement::{GoalShape, adapt_source_to_current_door, current_door_for_route_source};
use super::{EngineInner, LevelAssets};
use crate::ai::{DoorCombatInfo, Position, Stimulus, StimulusType};
use crate::coordinates::{MapPoint, MapVec};
use crate::element::{Command, Entity, EntityId};
use crate::order::OrderType;
use crate::sequence::{
    PendingCondolation, SequenceElement, SequenceId, take_goal_owner_terminal_provenance,
};

struct DamageParryHandoffDebugConfig {
    frame: u32,
    creation_order: u32,
}

fn damage_parry_handoff_debug_config() -> Option<&'static DamageParryHandoffDebugConfig> {
    static CONFIG: std::sync::OnceLock<Option<DamageParryHandoffDebugConfig>> =
        std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            std::env::var_os("PARITY_DEBUG_DAMAGE_PARRY_HANDOFF")?;
            let parse = |name: &str| {
                let raw = std::env::var(name).unwrap_or_else(|_| {
                    panic!("{name} is required when damage-parry handoff debugging is enabled")
                });
                raw.parse::<u32>().unwrap_or_else(|error| {
                    panic!("invalid {name}={raw:?} for damage-parry handoff diagnostic: {error}")
                })
            };
            Some(DamageParryHandoffDebugConfig {
                frame: parse("PARITY_DEBUG_DAMAGE_PARRY_HANDOFF_FRAME"),
                creation_order: parse("PARITY_DEBUG_DAMAGE_PARRY_HANDOFF_CREATION_ORDER"),
            })
        })
        .as_ref()
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum AttentiveModeCaller {
    AiOwnerEffect,
    ConsoleCheat,
    Unclassified,
}

struct AttentiveModeCallerDebugConfig {
    frame: u32,
    creation_order: u32,
}

fn attentive_mode_caller_debug_config() -> Option<&'static AttentiveModeCallerDebugConfig> {
    static CONFIG: std::sync::OnceLock<Option<AttentiveModeCallerDebugConfig>> =
        std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            std::env::var_os("PARITY_DEBUG_ATTENTIVE_MODE_CALLER")?;
            let parse = |name: &str| {
                let raw = std::env::var(name).unwrap_or_else(|_| {
                    panic!("{name} is required when attentive-mode caller debugging is enabled")
                });
                raw.parse::<u32>().unwrap_or_else(|error| {
                    panic!("invalid {name}={raw:?} for attentive-mode caller diagnostic: {error}")
                })
            };
            Some(AttentiveModeCallerDebugConfig {
                frame: parse("PARITY_DEBUG_ATTENTIVE_MODE_CALLER_FRAME"),
                creation_order: parse("PARITY_DEBUG_ATTENTIVE_MODE_CALLER_CREATION_ORDER"),
            })
        })
        .as_ref()
}

fn goal_owner_handoff_debug_frame_matches(frame: u32) -> bool {
    if std::env::var_os("PARITY_DEBUG_GOAL_OWNER_HANDOFF").is_none() {
        return false;
    }
    let value = std::env::var("PARITY_DEBUG_GOAL_OWNER_FRAME").unwrap_or_else(|_| {
        panic!("PARITY_DEBUG_GOAL_OWNER_HANDOFF requires PARITY_DEBUG_GOAL_OWNER_FRAME=FRAME")
    });
    let expected = value
        .parse::<u32>()
        .unwrap_or_else(|error| panic!("invalid PARITY_DEBUG_GOAL_OWNER_FRAME={value:?}: {error}"));
    frame == expected
}

#[cfg(test)]
thread_local! {
    static CONDOLATION_CARD_TRACE: std::cell::RefCell<Option<Vec<(EntityId, Command)>>> =
        const { std::cell::RefCell::new(None) };
    static CONDOLATION_STIMULUS_TRACE: std::cell::RefCell<Option<Vec<(EntityId, StimulusType)>>> =
        const { std::cell::RefCell::new(None) };
    static CONDOLATION_NESTED_TERMINATION: std::cell::RefCell<Option<(EntityId, StimulusType, SequenceId, usize)>> =
        const { std::cell::RefCell::new(None) };
    static OWNER_BOUNDARY_RESUME_TRACE: std::cell::RefCell<Option<Vec<EntityId>>> =
        const { std::cell::RefCell::new(None) };
    static STRANGLE_CONDOLATION_TRACE: std::cell::RefCell<Option<Vec<&'static str>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn capture_condolation_cards<T>(f: impl FnOnce() -> T) -> (T, Vec<(EntityId, Command)>) {
    CONDOLATION_CARD_TRACE.with(|trace| {
        assert!(trace.borrow_mut().replace(Vec::new()).is_none());
    });
    let result = f();
    let observed = CONDOLATION_CARD_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .take()
            .expect("condolation card trace remains installed")
    });
    (result, observed)
}

#[cfg(test)]
fn observe_condolation_card(owner: EntityId, command: Command) {
    CONDOLATION_CARD_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.push((owner, command));
        }
    });
}

#[cfg(test)]
pub(super) fn capture_strangle_condolation_order<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<&'static str>) {
    STRANGLE_CONDOLATION_TRACE.with(|trace| {
        assert!(trace.borrow_mut().replace(Vec::new()).is_none());
    });
    let result = f();
    let observed = STRANGLE_CONDOLATION_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .take()
            .expect("strangle condolation trace remains installed")
    });
    (result, observed)
}

#[cfg(test)]
pub(super) fn observe_strangle_condolation_step(step: &'static str) {
    STRANGLE_CONDOLATION_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.push(step);
        }
    });
}

#[cfg(test)]
pub(super) fn capture_condolation_stimuli<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<(EntityId, StimulusType)>) {
    CONDOLATION_STIMULUS_TRACE.with(|trace| {
        assert!(trace.borrow_mut().replace(Vec::new()).is_none());
    });
    let result = f();
    let observed = CONDOLATION_STIMULUS_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .take()
            .expect("condolation trace remains installed")
    });
    (result, observed)
}

#[cfg(test)]
fn observe_condolation_stimulus(owner: EntityId, stimulus: StimulusType) {
    CONDOLATION_STIMULUS_TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.push((owner, stimulus));
        }
    });
}

#[cfg(test)]
pub(super) fn install_condolation_nested_termination(
    owner: EntityId,
    stimulus: StimulusType,
    seq_id: SequenceId,
    elem_idx: usize,
) {
    CONDOLATION_NESTED_TERMINATION.with(|hook| {
        assert!(
            hook.borrow_mut()
                .replace((owner, stimulus, seq_id, elem_idx))
                .is_none(),
            "nested-condolation test hook must not already be installed"
        );
    });
}

#[cfg(test)]
pub(super) fn capture_owner_boundary_resumes<T>(f: impl FnOnce() -> T) -> (T, Vec<EntityId>) {
    OWNER_BOUNDARY_RESUME_TRACE.with(|trace| {
        assert!(trace.borrow_mut().replace(Vec::new()).is_none());
    });
    let result = f();
    let observed = OWNER_BOUNDARY_RESUME_TRACE.with(|trace| {
        trace
            .borrow_mut()
            .take()
            .expect("owner-boundary resume trace remains installed")
    });
    (result, observed)
}

#[cfg(test)]
fn take_condolation_nested_termination(
    owner: EntityId,
    stimulus: StimulusType,
) -> Option<(SequenceId, usize)> {
    CONDOLATION_NESTED_TERMINATION.with(|hook| {
        let mut hook = hook.borrow_mut();
        if hook
            .as_ref()
            .is_some_and(|(expected_owner, expected_stimulus, _, _)| {
                *expected_owner == owner && *expected_stimulus == stimulus
            })
        {
            hook.take()
                .map(|(_, _, seq_id, elem_idx)| (seq_id, elem_idx))
        } else {
            None
        }
    })
}

impl EngineInner {
    fn trace_damage_parry_handoff(
        &self,
        phase: &str,
        card: PendingCondolation,
        cross_postponed: Option<(SequenceId, usize)>,
    ) {
        let Some(config) = damage_parry_handoff_debug_config() else {
            return;
        };
        if card.command != Command::ReceiveSwordDamage || config.frame != self.control.frame_counter
        {
            return;
        }
        let creation_order = self.world.original_creation_order(card.owner);
        if creation_order != config.creation_order {
            return;
        }

        let manager = &self.orders.sequence_manager;
        let selected = manager.current_element_for_actor(card.owner);
        let deferred = manager
            .deferred_elements_to_go()
            .into_iter()
            .filter(|(seq_id, elem_idx)| {
                manager
                    .get_element(*seq_id, *elem_idx)
                    .is_some_and(|element| element.owner == Some(card.owner))
            })
            .collect::<Vec<_>>();
        eprintln!(
            "PARITY_DAMAGE_PARRY_HANDOFF frame={} phase={} owner={} owner_co={} terminal_seq={} terminal_elem={} terminal_state={:?} selected={selected:?} cross_postponed={cross_postponed:?} deferred={deferred:?}",
            self.control.frame_counter,
            phase,
            card.owner.index(),
            creation_order,
            card.seq_id.0,
            card.elem_idx,
            card.terminal_state,
        );
        for sequence in manager.sequences_iter() {
            for (elem_idx, element) in sequence.elements.iter().enumerate() {
                if element.owner != Some(card.owner) {
                    continue;
                }
                eprintln!(
                    "PARITY_DAMAGE_PARRY_HANDOFF frame={} phase=element owner={} owner_co={} seq={} elem={} id={} command={:?} level={} state={:?} priority={:?} legacy_v48={} postponed={:?} cross_postponed={:?} first_order={:?} last_order={:?}",
                    self.control.frame_counter,
                    card.owner.index(),
                    creation_order,
                    sequence.id.0,
                    elem_idx,
                    element.id,
                    element.command,
                    element.command_level,
                    element.state,
                    element.priority,
                    element.legacy_v48.is_some(),
                    element.postponed_element_index,
                    element.cross_postponed,
                    element
                        .orders
                        .front()
                        .map(|order| (order.order_type, order.done)),
                    element
                        .orders
                        .back()
                        .map(|order| (order.order_type, order.done)),
                );
            }
        }
    }

    /// Request a soldier enter or leave attentive mode, launching the
    /// appropriate transition-animation sequence element.
    ///
    /// Early-exits when the target matches `will_be_attentive`,
    /// otherwise picks `EnterAttentiveMode` / `LeaveAttentiveMode` /
    /// `LeaveAttentiveModeOfficer` and launches it as a
    /// `SequenceElement`.  The officer variant is only chosen when
    /// `fast_variant` is set *and* the sprite actually has the officer
    /// transition animation.
    pub(crate) fn set_soldier_attentive_mode(
        &mut self,
        entity_id: EntityId,
        target: bool,
        fast_variant: bool,
    ) {
        self.set_soldier_attentive_mode_from(
            entity_id,
            target,
            fast_variant,
            AttentiveModeCaller::Unclassified,
        );
    }

    pub(crate) fn set_soldier_attentive_mode_from(
        &mut self,
        entity_id: EntityId,
        target: bool,
        fast_variant: bool,
        caller: AttentiveModeCaller,
    ) {
        // Keep the diagnostic master/frame gates ahead of every added entity
        // or creation-order read. Normal behavior below retains its original
        // reads and control flow when the diagnostic is disabled.
        let debug_creation_order = attentive_mode_caller_debug_config().and_then(|config| {
            (self.control.frame_counter == config.frame)
                .then(|| self.world.original_creation_order(entity_id))
                .filter(|creation_order| *creation_order == config.creation_order)
        });
        let (will_be_attentive, has_officer_anim) = match self.world.entities.get(entity_id) {
            Some(Entity::Soldier(s)) => (
                s.npc
                    .ai_brain
                    .enemy()
                    .map(|e| e.will_be_attentive)
                    .unwrap_or(false),
                s.element
                    .sprite
                    .has_animation(OrderType::TransitionWaitingAlertedWaitingUprightOfficer),
            ),
            _ => return,
        };

        if let Some(creation_order) = debug_creation_order {
            let attentive = self
                .world
                .entities
                .get(entity_id)
                .and_then(Entity::enemy_ai)
                .expect("attentive-mode caller diagnostic target lost enemy AI")
                .attentive;
            eprintln!(
                "ATTENTIVE_MODE_CALLER frame={} owner={} creation_order={} phase=before caller={caller:?} target={target} fast_variant={fast_variant} attentive={attentive} will_be_attentive={will_be_attentive}",
                self.control.frame_counter,
                entity_id.index(),
                creation_order,
            );
        }

        if target == will_be_attentive {
            if let Some(creation_order) = debug_creation_order {
                eprintln!(
                    "ATTENTIVE_MODE_CALLER frame={} owner={} creation_order={} phase=after caller={caller:?} target={target} launched_sequence=None attentive={} will_be_attentive={will_be_attentive}",
                    self.control.frame_counter,
                    entity_id.index(),
                    creation_order,
                    self.world
                        .entities
                        .get(entity_id)
                        .and_then(Entity::enemy_ai)
                        .expect("attentive-mode caller diagnostic target lost enemy AI")
                        .attentive,
                );
            }
            return;
        }

        let command = if target {
            Command::EnterAttentiveMode
        } else if fast_variant && has_officer_anim {
            Command::LeaveAttentiveModeOfficer
        } else {
            Command::LeaveAttentiveMode
        };

        let launched_sequence =
            self.launch_element(SequenceElement::new(1, command, Some(entity_id)));

        if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(entity_id)
            && let Some(enemy) = s.npc.ai_brain.enemy_mut()
        {
            enemy.will_be_attentive = target;
        }
        if let Some(creation_order) = debug_creation_order {
            let enemy = self
                .world
                .entities
                .get(entity_id)
                .and_then(Entity::enemy_ai)
                .expect("attentive-mode caller diagnostic target lost enemy AI");
            eprintln!(
                "ATTENTIVE_MODE_CALLER frame={} owner={} creation_order={} phase=after caller={caller:?} target={target} launched_sequence={} attentive={} will_be_attentive={}",
                self.control.frame_counter,
                entity_id.index(),
                creation_order,
                launched_sequence.0,
                enemy.attentive,
                enemy.will_be_attentive,
            );
        }
    }

    /// Translate `ReceiveWaspSting` during sequence dispatch.  Books
    /// the `GettingFreeFromWasp` turning animation with a random
    /// rotation offset (-8..=+8), says the wasp-sting remark, and
    /// fires the `EventWasp` AI stimulus.
    ///
    /// The original queues `bee_time` struggle orders, doubled while
    /// smelling apple. The order-completion side channel below re-arms
    /// each cycle and keeps the sequence element in progress until the
    /// last cycle finishes.
    pub(super) fn dispatch_receive_wasp_sting(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    ) {
        let Some(entity) = self.world.entities.get(owner) else {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return;
        };
        if !entity.is_soldier() {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return;
        }

        // Random rotation offset, applied to direction_goal so the
        // soldier rotates during the animation.  Range is `rand(0..17) - 8`.
        let rotation =
            crate::sim_rng::i32(sim, crate::sim_rng::RngSite::SoldierFreedRotation, 0..17) - 8;
        let new_goal = {
            let current_goal = i16::from(entity.position_iface().get_direction_goal());
            (current_goal + rotation as i16).rem_euclid(16)
        };

        // On-ladder soldiers fall off the ladder before the struggle begins.
        let on_ladder = entity.element_data().posture == crate::element::Posture::OnLadder;

        // bee_time struggle cycles, doubled when apple-smelling.  Each
        // cycle is one `GettingFreeFromWasp` animation; the sequence
        // element stays in-progress until the last one finishes.
        let bee_time = entity
            .soldier_data()
            .and_then(|s| assets.profile_manager.get_soldier(s.soldier_profile_index))
            .map(|p| {
                let base = p.bee_time.max(1) as u32;
                if entity
                    .soldier_data()
                    .map(|s| s.apple_smell > 0)
                    .unwrap_or(false)
                {
                    base * 2
                } else {
                    base
                }
            })
            .unwrap_or(1)
            .min(u16::MAX as u32) as u16;

        // Drop the immutable entity borrow before any mutable self calls.
        let _ = entity;

        // Process the ladder fall before the wasp-struggle orders are
        // queued; the soldier needs to leave the lift first and the
        // animation queue would otherwise be clobbered.
        if on_ladder {
            self.translate_ladder_wall_fall(assets, owner, (seq_id, elem_idx));
        }

        // Book the first struggle animation onto the wasp-sting
        // sequence element so `current_order_for_actor` picks it up.
        // Each cycle's completion hook (`WaspStruggleCycle`) decides
        // whether to re-push the next cycle or terminate the element.
        if let Some(entity) = self.world.entities.get_mut(owner) {
            if entity.actor_data().is_some() {
                entity.position_iface_mut().set_direction(
                    crate::position_interface::Direction::from_raw(new_goal as i32),
                );
            }
            if let Some(npc) = entity.npc_data_mut() {
                npc.wasp_victim = true;
            }
        }
        let order = crate::order::Order::new(
            OrderType::GettingFreeFromWasp,
            0.0,
            0.0,
            self.orders.allocate_order_id(),
        )
        .with_completion(crate::order::OrderCompletion::WaspStruggleCycle {
            cycles_remaining: bee_time,
        });
        self.orders
            .sequence_manager
            .push_order_on(seq_id, elem_idx, order);

        // Queue the EventWasp AI stimulus.
        self.dispatch_ai_stimulus(owner, Stimulus::new(StimulusType::EventWasp));

        self.orders
            .sequence_manager
            .element_in_progress(seq_id, elem_idx);
    }

    /// Drain and dispatch `SendCondolationCard` notifications queued by
    /// the sequence manager since the last call.  Invoked by the tick
    /// loop after `hourglass` so every sequence-element terminal state
    /// change (Terminated / Interrupted / Impossible) fires its
    /// per-entity cleanup in a single pass.
    ///
    /// Notifications are queued (rather than fired inline) to avoid
    /// re-entrant borrows; this method drains the queue.
    pub(super) fn dispatch_condolations(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        self.dispatch_condolations_in_script_driver(sim, assets, &mut Vec::new())
            .unwrap_or_else(|error| panic!("condolation dispatch failed: {error:?}"));
    }

    /// Close `SetState`'s owner-card/`Ready()` stack without dropping the
    /// active VM frames that made the state change.
    pub(super) fn dispatch_condolations_in_script_driver(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
    ) -> Result<(), crate::engine::script::ScriptDriverError> {
        let mut pending: std::collections::VecDeque<_> = self
            .orders
            .sequence_manager
            .drain_pending_condolations()
            .into();
        while let Some(dispatch) = pending.pop_front() {
            let owner = dispatch.card.owner;
            let from_halt = dispatch.card.from_halt;
            let diagnostic = damage_parry_handoff_debug_config()
                .map(|_| (dispatch.card, dispatch.cross_postponed_successor()));
            if let Some((card, cross)) = diagnostic {
                self.trace_damage_parry_handoff("before_card", card, cross);
            }
            self.send_condolation_card(sim, dispatch.card, assets);
            // A Halt card performs actor/human cleanup, but the NPC override
            // returns before Think. It cannot causally produce any of these
            // continuations, so do not let its empty callback steal
            // replacement work which the Halt caller queued beforehand.
            if !from_halt {
                // SendCondolationCard re-enters Think for this owner directly.
                // Do not resolve unrelated actors' prepared movement forecasts
                // while closing that native owner-local call stack.
                self.drain_self_stimuli_for_npc_without_forecast(sim, owner, assets);
                self.dispatch_pending_waypoint_script_for_owner(sim, owner, assets);
                self.dispatch_synchronous_owner_moves(sim, assets, owner, active_scripts)?;
            }
            self.orders
                .sequence_manager
                .finish_pending_condolation(dispatch);
            if let Some((card, cross)) = diagnostic {
                self.trace_damage_parry_handoff("after_finish", card, cross);
            }
            // Original `RHSequenceElement::SetState` resumes directly after
            // `SendCondolationCard`: `Ready()` can advance the sequence and
            // `RegisterSequenceElementToGo` immediately calls
            // `ExecutedImmediately()` for Timer/LockUser/etc.  Keep that work
            // inside this exact SetState stack frame, before another
            // condolence card or NPC Hourglass slot can run.
            self.drain_script_synchronous_actions(sim, assets, active_scripts)?;
            if let Some((card, cross)) = diagnostic {
                self.trace_damage_parry_handoff("after_sync_drain", card, cross);
            }

            for nested in self
                .orders
                .sequence_manager
                .drain_pending_condolations()
                .into_iter()
                .rev()
            {
                pending.push_front(nested);
            }
        }
        Ok(())
    }

    /// Complete pending condolence stack frames after an NPC Think call.
    /// Cascading `SetState` calls can cross owners, so filtering by the
    /// originating NPC would strand a nested `SendCondolationCard` until
    /// later in the frame.  Once this synchronous boundary is entered, all
    /// queued cards must therefore drain depth-first.
    pub(super) fn dispatch_condolations_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        _npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        self.dispatch_condolations(sim, assets);
    }

    /// Close terminal cards for one live actor owner, then follow newly
    /// generated cross-owner cards depth-first. Foreign cards that predate
    /// this owner slot remain queued for their established boundary.
    pub(super) fn dispatch_condolations_for_owner_boundary(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
        assets: &LevelAssets,
    ) -> Vec<(crate::sequence::SequenceId, usize)> {
        let preexisting = self.orders.sequence_manager.drain_pending_condolations();
        let (roots, foreign_backlog): (Vec<_>, Vec<_>) = preexisting
            .into_iter()
            .partition(|dispatch| dispatch.card.owner == owner);
        let mut resumed_cross_successors = Vec::new();
        for root in roots {
            self.close_owner_boundary_condolation(sim, assets, root, &mut resumed_cross_successors);
        }
        self.orders
            .sequence_manager
            .restore_pending_condolations(foreign_backlog);
        resumed_cross_successors
    }

    fn close_owner_boundary_condolation(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        dispatch: crate::sequence::PendingCondolationDispatch,
        resumed_cross_successors: &mut Vec<(crate::sequence::SequenceId, usize)>,
    ) {
        let card_owner = dispatch.card.owner;
        let from_halt = dispatch.card.from_halt;
        let diagnostic = damage_parry_handoff_debug_config()
            .map(|_| (dispatch.card, dispatch.cross_postponed_successor()));
        if let Some((card, cross)) = diagnostic {
            self.trace_damage_parry_handoff("owner_boundary_before_card", card, cross);
        }
        self.send_condolation_card(sim, dispatch.card, assets);
        if !from_halt {
            self.drain_self_stimuli_for_npc_without_forecast(sim, card_owner, assets);
            self.dispatch_pending_waypoint_script_for_owner(sim, card_owner, assets);
        }

        // A condolence Think can issue GoTo for the next patrol leg. C++
        // LaunchSequenceElement immediately reaches the owner's Instruct
        // path inside SendCondolationCard, before the terminating element's
        // Ready() continuation resumes. Promote only this owner's newly
        // queued move and drive its exact deferred InstructOwner action at
        // the same boundary. Other owners' FIFO positions remain untouched.
        let mut active_scripts = Vec::new();
        if !from_halt {
            self.dispatch_synchronous_owner_moves(sim, assets, card_owner, &mut active_scripts)
                .unwrap_or_else(|error| {
                    panic!(
                        "condolation owner {} synchronous Move dispatch failed: {error:?}",
                        card_owner.index()
                    )
                });
        }

        // A SetState reached re-entrantly from SendCondolationCard belongs
        // inside that call. Close those cards before resuming this outer
        // SetState at Ready()/cascade.
        for nested in self.orders.sequence_manager.drain_pending_condolations() {
            self.close_owner_boundary_condolation(sim, assets, nested, resumed_cross_successors);
        }

        #[cfg(test)]
        OWNER_BOUNDARY_RESUME_TRACE.with(|trace| {
            if let Some(trace) = trace.borrow_mut().as_mut() {
                trace.push(card_owner);
            }
        });
        self.orders
            .sequence_manager
            .finish_pending_condolation(dispatch);
        if let Some((card, cross)) = diagnostic {
            self.trace_damage_parry_handoff("owner_boundary_after_finish", card, cross);
        }

        // `StartPostponedSequenceElement` only calls
        // `RegisterSequenceElementToGo`. Ordinary released work therefore
        // remains in the manager FIFO until `RHSequenceManager::Hourglass`;
        // only commands handled by registration itself may run inline.
        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
            .unwrap_or_else(|error| {
                panic!(
                    "owner-boundary condolation for {} failed to drain its synchronous successor: {error:?}",
                    card_owner.index()
                )
            });
        if let Some((card, cross)) = diagnostic {
            self.trace_damage_parry_handoff("owner_boundary_after_sync_drain", card, cross);
        }

        for nested in self.orders.sequence_manager.drain_pending_condolations() {
            self.close_owner_boundary_condolation(sim, assets, nested, resumed_cross_successors);
        }
    }

    /// Promote and synchronously instruct Move elements launched re-entrantly
    /// by an owner's condolence-card Think call. In the Original this entire
    /// chain remains inside `SendCondolationCard`; unrelated owners keep their
    /// established queue positions.
    pub(super) fn dispatch_synchronous_owner_moves(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        active_scripts: &mut Vec<crate::engine::script::ActiveScriptCall>,
    ) -> Result<(), crate::engine::script::ScriptDriverError> {
        for sequence_id in self.drain_pending_move_requests_for_owner(sim, owner) {
            let action = self
                .orders
                .sequence_manager
                .take_deferred_owner_action(owner, sequence_id, 0)
                .unwrap_or_else(|detail| {
                    panic!(
                        "condolation owner {} Move dispatch failed: {detail}",
                        owner.index()
                    )
                });
            if let Some(action) = action {
                self.dispatch_script_synchronous_action(sim, assets, action, active_scripts)?;
            }
        }
        Ok(())
    }

    /// Dispatch a single `SendCondolationCard` to the owner entity.
    ///
    /// When a sequence element reaches a terminal state, walk the
    /// remaining chain via `is_last_real_action`; if nothing
    /// meaningful follows (and the AI isn't inside a `Halt()` call),
    /// dispatch a command-specific stimulus back to the owner so the
    /// AI state machine can advance.  That dispatch is what unsticks
    /// substates like `DefaultOnPostLookingSidewards`, whose only exit
    /// is an `EventDone` stimulus after the `LookLeft` / `LookRight`
    /// sequence completes.
    fn send_condolation_card(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        card: PendingCondolation,
        assets: &LevelAssets,
    ) {
        use crate::sequence::SequenceState;
        let PendingCondolation {
            owner,
            command,
            terminal_state,
            seq_id,
            elem_idx,
            was_selected,
            from_halt,
            postponed_successor_pending,
            cancel_path_request_owner,
        } = card;
        let frame = self.control.frame_counter;
        let goal_owner_provenance = take_goal_owner_terminal_provenance(owner, seq_id, elem_idx)
            .filter(|_| goal_owner_handoff_debug_frame_matches(frame));

        #[cfg(test)]
        observe_condolation_card(owner, command);

        if let Some(path_owner) = cancel_path_request_owner {
            // RHSequenceElementMovement::MaybeCancelPathRequest runs before
            // the base-class SendCondolationCard call. Keep cancellation at
            // that same callback boundary so no linked-Seek or owner Think
            // callback can observe the stale request.
            self.world.pathfinder.cancel_requests_for(path_owner);
            self.orders
                .pending_path_requests
                .cancel_for_owner(path_owner);
            self.orders
                .failed_path_requests
                .retain(|request| request.owner != path_owner);
        }

        // Snapshot the owner's posture for the `is_very_very_busy` check
        // below without holding a mutable borrow on `self.world.entities` — so
        // the Impossible arm can read `self.orders.sequence_manager` without a
        // split-borrow conflict, and the final state mutation below can
        // use a fresh `get_mut`.
        if !self.world.entities.get(owner).is_some() {
            return;
        }

        // PC override.  Runs before the human-base chain, so its
        // STRANGLE / TAKE_CORPSE antagonist-side effects fire
        // regardless of the `is_last_real_action` / `from_halt` gates
        // below (those gate the per-owner stimulus dispatch, which is
        // NPC-only — the PC has no `ai_controller` for
        // `fire_self_stimulus` to land on).
        if self.world.entities.get(owner).is_some_and(|e| e.is_pc()) {
            self.send_condolation_card_pc(sim, owner, command, seq_id, elem_idx, assets);
        }

        // Soldier override: ReceiveWaspSting termination clears the
        // wasp-victim flag.  The human-base cleanup runs before the
        // halt-method guard, so it fires whether or not `from_halt`
        // is set.
        if matches!(command, Command::ReceiveWaspSting)
            && let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(npc) = entity.npc_data_mut()
        {
            npc.wasp_victim = false;
        }

        // Actor-base `SendCondolationCard` clears the sprite movement goal
        // and detaches `mpSequenceElement` / `mpOrder` when the card belongs
        // to the actor's currently selected element. This happens before the
        // NPC override checks `mbInsideHaltMethod`. For movement, Rust's
        // `active_movement` is the exact selected element identity; ordinary
        // order exhaustion performs the same cleanup synchronously in
        // `do_next_order`, before detaching that tracker.
        let selected_element = self
            .orders
            .sequence_manager
            .current_element_for_actor(owner);
        // `was_selected` is captured at SetState, the exact Original
        // boundary at which the card's element is compared against the
        // actor's selection. Rust dispatches the card later, so replacing it
        // with a fresh selection lookup would reinterpret history and strand
        // the old movement goal: by the time the card runs, the terminal
        // element is no longer registered as selected even though it still
        // was when it left `InProgress`.
        let detaches_selected_goal = was_selected;
        // The order pointer needs a stricter gate than the goal. Original
        // clears goal and order together, at an instant when no successor can
        // yet be selected — the successor republishes both strictly
        // afterwards. Rust preserves that ordering for the goal, which is
        // still installed late by the movement instruction, but publishes the
        // order pointer eagerly at instruct time. A card deferred past an
        // incoming element — a lazily installed Wait, or a recursive
        // successor — would therefore clear a pointer the newcomer has
        // already published, leaving the actor orderless mid-action. Detach
        // only the terminal element's own leftovers.
        //
        // Replacement arbitration covers the narrower case where the incoming
        // element is selected before the outgoing one is interrupted, and
        // explicitly records `was_selected = false` for it.
        let successor_already_selected =
            selected_element.is_some_and(|selected| selected != (seq_id, usize::from(elem_idx)));
        let detaches_selected_order = detaches_selected_goal && !successor_already_selected;
        let mut cleared_selected_goal = false;
        if let Some(provenance) = &goal_owner_provenance {
            let actor_state = self.world.entities.get(owner).and_then(|entity| {
                entity.actor_data().map(|actor| {
                    (
                        actor.action_state,
                        actor.active_movement,
                        actor.installed_order,
                    )
                })
            });
            let position_state = self.world.entities.get(owner).map(|entity| {
                (
                    entity.position_iface().map_goal(),
                    entity.position_iface().is_moving(),
                )
            });
            let translating = self.orders.sequence_manager.goal_owner_debug_translating();
            eprintln!(
                "[GOAL_OWNER frame={frame} owner={owner:?} stage=before_condolation_cleanup site={} seq={seq_id:?} elem={elem_idx} command={command:?} terminal={terminal_state:?} was_selected={was_selected} terminal_selected={:?} terminal_translating={:?} dispatch_selected={selected_element:?} dispatch_translating={translating:?} actor={actor_state:?} position={position_state:?}]",
                provenance.site, provenance.selected, provenance.translating,
            );
        }
        if let Some(entity) = self.world.entities.get_mut(owner) {
            let active_movement_matches = entity.actor_data().is_some_and(|actor| {
                actor.active_movement.sequence_id == Some(seq_id)
                    && actor.active_movement.element_index == elem_idx as usize
            });

            // Actor-base cleanup keys goal clearing to the selected
            // `mpSequenceElement`, independently of whether that element is
            // tracked as movement. A synchronous AssertPosition can select
            // and terminate without ever becoming `active_movement`.
            let clears_goal =
                detaches_selected_goal || (active_movement_matches && selected_element.is_none());
            tracing::trace!(
                target: "parity_owner_handoff",
                frame,
                ?owner,
                ?seq_id,
                elem_idx,
                ?command,
                ?terminal_state,
                was_selected,
                ?selected_element,
                active_movement_matches,
                clears_goal,
                detaches_selected_order,
                goal = ?entity.position_iface().map_goal(),
                "condolation card owner cleanup"
            );
            if clears_goal {
                entity
                    .position_iface_mut()
                    .set_map_goal(crate::coordinates::MapPoint::ZERO);
                cleared_selected_goal = true;
            }

            // `installed_order` is Rust's mirror of the selected `mpOrder`.
            // Original clears that pointer before entering the NPC
            // condolence callback, so every recursive Think must observe
            // NONANIMATION_END rather than the completed sprite transition.
            if detaches_selected_order && let Some(actor) = entity.actor_data_mut() {
                tracing::trace!(
                    ?owner,
                    ?seq_id,
                    elem_idx,
                    ?command,
                    ?selected_element,
                    detached = ?actor.installed_order,
                    "condolation card detaching installed order"
                );
                actor.installed_order = None;
            }

            // Rust's movement tracker is separate from Original's selected
            // element pointer. Detach it only when this card names that
            // movement, even if an incoming element is already selected.
            if active_movement_matches && let Some(actor) = entity.actor_data_mut() {
                actor.active_movement.clear();
            }

            // Bow execution is tracked separately from the selected element
            // so its animation phase can release the projectile. Original has
            // no independent tracker: interrupting/impossibilizing the
            // sequence deletes its orders, and the actor has already selected
            // any incoming replacement before this condolence callback. Clear
            // the Rust handle by exact element identity even when the old bow
            // element is no longer selected, or a later valid shot will be
            // rejected as though the cancelled shot were still active.
            if let Some(actor) = entity.actor_data_mut()
                && actor.active_shot.sequence_id == Some(seq_id)
                && actor.active_shot.element_index == elem_idx as usize
            {
                actor.active_shot.clear();
            }

            // Ability execution is likewise an implementation-side mirror of
            // the selected C++ element/order. Once that exact element sends
            // its condolence card, no independent ability may survive to
            // reject a later valid command. This is especially observable
            // when an ability is interrupted after its DONE effect but before
            // the sprite's TERMINATED edge.
            if let Some(actor) = entity.actor_data_mut()
                && actor.active_ability.sequence_id == Some(seq_id)
                && actor.active_ability.element_index == elem_idx as usize
            {
                let kind = actor.active_ability.kind;
                actor.active_ability.clear();
                if kind == Some(crate::movement::AbilityKind::Listen) {
                    actor.listen_phase = crate::element::ListenPhase::Inactive;
                    actor.listen_wait_time = 0;
                } else if kind == Some(crate::movement::AbilityKind::ReceivePurse) {
                    actor.receive_purse_phase = crate::element::ReceivePursePhase::Inactive;
                }
            }
        }
        if detaches_selected_order {
            // Same C++ statement pair as the `installed_order` detach above:
            // `mpSequenceElement = NULL` alongside `mpOrder = NULL`
            // (`RHelementactor.cpp:6698-6700`). Rust's in-progress registry
            // drops the element on its own terminal transition, but a card
            // raised from inside the element's own `Translate` still holds the
            // translation selection, and everything that runs later in that
            // same `Translate` — notably the `Wait()` that
            // `RHPathFinder::AddPathRequest` launches right after `Stop()`
            // (`RHpathfinder.cpp:464-465`) — must arbitrate against a cleared
            // selection instead of the element that just died.
            self.orders
                .sequence_manager
                .clear_translating_element_if_selected(owner, seq_id, usize::from(elem_idx));
        }
        if cleared_selected_goal {
            // A queued replacement may carry the outgoing movement's goal
            // across Rust's eager halt. Once a later selected element has
            // performed Original's actor-base goal cleanup, that snapshot is
            // stale and must not be resurrected when the replacement finally
            // reaches its MoveWaiting/path-request instruction.
            self.orders
                .sequence_manager
                .clear_retained_movement_goals_for_actor(owner);
        }
        if let Some(provenance) = &goal_owner_provenance {
            let selected = self
                .orders
                .sequence_manager
                .current_element_for_actor(owner);
            let translating = self.orders.sequence_manager.goal_owner_debug_translating();
            let actor_state = self.world.entities.get(owner).and_then(|entity| {
                entity.actor_data().map(|actor| {
                    (
                        actor.action_state,
                        actor.active_movement,
                        actor.installed_order,
                    )
                })
            });
            let position_state = self.world.entities.get(owner).map(|entity| {
                (
                    entity.position_iface().map_goal(),
                    entity.position_iface().is_moving(),
                )
            });
            eprintln!(
                "[GOAL_OWNER frame={frame} owner={owner:?} stage=after_condolation_cleanup site={} seq={seq_id:?} elem={elem_idx} command={command:?} terminal={terminal_state:?} was_selected={was_selected} selected={selected:?} translating={translating:?} actor={actor_state:?} position={position_state:?} cleared_goal={cleared_selected_goal}]",
                provenance.site,
            );
        }

        // When a movement element with the MAP flag terminates, toggle
        // `active` / `in_honolulu` / anti-collision based on whether
        // the actor's current map position is inside the playable map
        // box.  Without this the reinforcement-door flow (PASS_DOOR |
        // MAP) leaves the actor sitting on the wrong side of the
        // activate/honolulu split — script-driven `SetActorLocation`
        // is the only other path that cleans it up.  Runs unconditionally,
        // regardless of `from_halt` / the `is_last_real_action` Think
        // gate below.
        let map_flag_terminated = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx as usize)
            .map(|e| {
                matches!(
                    e.data,
                    crate::sequence::SequenceElementData::Movement { flags, .. }
                        if flags.contains(crate::sequence::MoveFlags::MAP)
                )
            })
            .unwrap_or(false);
        if map_flag_terminated && let Some(entity) = self.world.entities.get_mut(owner) {
            let pos = entity.element_data().position_map();
            // RHElementActor::SendCondolationCard tests GetBoxMap(), whose
            // authored bounds exclude the four grid rows reserved as loader
            // padding.  Grid-addressability is therefore intentionally not
            // the activation boundary for a completed RHMOVE_MAP.
            let inside_map = self.world.fast_grid.level.map_bbox.contains_point(pos);
            // Re-borrow mutably (the read above released the borrow).
            if let Some(entity) = self.world.entities.get_mut(owner) {
                let ed = entity.element_data_mut();
                ed.active = inside_map;
                ed.in_honolulu = !inside_map;
                if inside_map {
                    ed.sprite.position_iface.set_anti_collision_on(true);
                }
            }
        }

        // Release the implementation-side latch before the strike's
        // EventDone re-enters ordinary swordfight AI. Otherwise that
        // reconsideration incorrectly believes a strike is still pending and
        // suppresses the next source-authorized proposal.
        if command.is_swordstrike()
            && matches!(
                terminal_state,
                SequenceState::Terminated | SequenceState::Interrupted
            )
            && let Some(crate::element::Entity::Soldier(soldier)) =
                self.world.entities.get_mut(owner)
            && let crate::element::AiBrain::Enemy(ai) = &mut soldier.npc.ai_brain
        {
            ai.pending_special_strike = false;
        }

        // Skip the per-command Think dispatch when further real
        // actions follow in the chain, or when we're tearing the
        // sequence down from inside a `Halt()` call.
        if from_halt
            || postponed_successor_pending
            || !self
                .orders
                .sequence_manager
                .is_last_real_action(seq_id, elem_idx as usize)
        {
            return;
        }

        // Decide which stimulus the owner should see based on the
        // command and terminal state.
        let stimulus = match command {
            // Motion commands — Terminated → ReachPoint, Impossible →
            // CouldntReachPoint.  Nothing for Interrupted (motion
            // dispatch has no Interrupted stimulus).
            Command::PassDoor | Command::Move | Command::MoveOk | Command::SitDown => {
                match terminal_state {
                    SequenceState::Terminated => Some(StimulusType::EventReachPoint),
                    SequenceState::Impossible => {
                        // Flip `was_busy` + take AILOCK_BUSY when the
                        // actor got stuck because it's "very very busy"
                        // — posture Flying / OnLadder / OnWall, or the
                        // active sequence element is a PassDoor / Fall.
                        // Mid-transition postures can't accept a fresh
                        // move, so the AI holds on AILOCK_BUSY until
                        // the posture clears.  The per-tick edge
                        // detector (`tick_npc_busy_edge_detect_for_npc`)
                        // handles the symmetric unlock when the busy
                        // state clears.
                        if self.is_very_very_busy(owner)
                            && let Some(ent) = self.world.entities.get_mut(owner)
                            && let Some(ai) = ent.ai_controller_mut()
                            && !ai.was_busy
                        {
                            ai.non_script_lock(crate::ai::AiLockFlags::BUSY);
                            ai.was_busy = true;
                        }
                        Some(StimulusType::EventCouldntReachPoint)
                    }
                    _ => None,
                }
            }

            // Generic action commands — Terminated or Interrupted →
            // EventDone, any other terminal state → EventImpossible.
            // Covers the long case list including LookLeft / LookRight
            // / LeanOut which is what the idle "look around" cascade
            // waits on.
            Command::DrinkWhisky
            | Command::KickLow
            | Command::Point
            | Command::FlyDoor
            | Command::Untie
            | Command::Fainted
            | Command::Recover
            | Command::Knee
            | Command::Turn
            | Command::TurnElement
            | Command::TurnFast
            | Command::SwordstrikeThrustA
            | Command::SwordstrikeThrustB
            | Command::SwordstrikeThrustC
            | Command::SwordstrikeThrustD
            | Command::SwordstrikeThrustE
            | Command::SwordstrikeThrustF
            | Command::SwordstrikeThrustG
            | Command::SwordstrikeThrustH
            | Command::SwordstrikeThrustI
            | Command::SwordstrikeTired
            | Command::StandUp
            | Command::SwordstrikeDown
            | Command::StopParrySword
            | Command::LookLeft
            | Command::LookRight
            | Command::LeanOut
            | Command::WakeUp
            | Command::ReceiveSwordDamage
            | Command::ReceiveHitDamage
            | Command::EquipBow
            | Command::EquipBowUp
            | Command::EquipBowDown
            | Command::RaiseBow
            | Command::ShootBow
            | Command::Take
            | Command::HitCmd
            | Command::SearchCmd
            | Command::DrinkAle
            | Command::LowerShield
            | Command::StartMenace
            | Command::EnterSwordfight => match terminal_state {
                SequenceState::Terminated | SequenceState::Interrupted => {
                    Some(StimulusType::EventDone)
                }
                _ => Some(StimulusType::EventImpossible),
            },

            // Smalltalk-combat commands — only Terminated / Interrupted
            // fire, no Impossible fallback.
            Command::SwordstrikeSmalltalkLeft
            | Command::SwordstrikeSmalltalkRight
            | Command::ParrySmalltalkLeft
            | Command::ParrySmalltalkRight => match terminal_state {
                SequenceState::Terminated | SequenceState::Interrupted => {
                    Some(StimulusType::EventDone)
                }
                _ => None,
            },

            // Plain Wait — EventDone only on clean termination.
            Command::Wait => match terminal_state {
                SequenceState::Terminated => Some(StimulusType::EventDone),
                _ => None,
            },

            // Wasp sting — fires EventWaspAway on every terminal state.
            Command::ReceiveWaspSting => match terminal_state {
                SequenceState::Terminated
                | SequenceState::Impossible
                | SequenceState::Interrupted => Some(StimulusType::EventWaspAway),
                _ => None,
            },

            _ => None,
        };

        #[cfg(test)]
        if let Some(st) = stimulus {
            observe_condolation_stimulus(owner, st);
            if let Some((seq_id, elem_idx)) = take_condolation_nested_termination(owner, st) {
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
        }

        if let Some(st) = stimulus
            && let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(ai) = entity.ai_controller_mut()
        {
            tracing::trace!(
                owner = owner.index(),
                ?command,
                ?terminal_state,
                stimulus = ?st,
                "send_condolation_card: fire EventDone/EventReachPoint to owner"
            );
            ai.outbox
                .reentrant
                .self_stimuli
                .push(crate::ai::QueuedSelfStimulus::new(
                    st,
                    crate::ai::SelfStimulusOrigin::Condolation,
                ));
        }
    }

    /// PC-specific arms of `send_condolation_card`.
    ///
    /// Two arms with antagonist side-effects:
    ///
    /// - **Strangle**: when a strangle element terminates, reset the
    ///   victim's AI/eye state.  Fires for both the success path
    ///   (victim about to be killed via ReceiveDamage — the Wait /
    ///   Unlock / SetView calls are dead-on-corpse but kept for
    ///   symmetry) and the abort/non-stranglable path (victim alive —
    ///   needs the AI un-wedged).
    ///
    /// - **TakeCorpse**: if the carrier was mid-grab (`pc.carried` set
    ///   but the carried's posture isn't yet `Carried`), force-drop
    ///   the partially-grabbed body.  `pc.carried` is set by
    ///   `begin_carry` at TakeCorpse init.
    fn send_condolation_card_pc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
        command: Command,
        seq_id: SequenceId,
        elem_idx: u16,
        assets: &LevelAssets,
    ) {
        match command {
            Command::StrangleCmd => {
                // Look up the antagonist NPC stored on the strangle
                // element's required `Interaction` data.
                let elem = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx as usize)
                    .unwrap_or_else(|| {
                        panic!(
                            "strangle condolation owner {owner:?} requires missing element {seq_id:?}/{elem_idx}"
                        )
                    });
                assert_eq!(
                    elem.command,
                    Command::StrangleCmd,
                    "strangle condolation owner {owner:?} reached wrong command on {seq_id:?}/{elem_idx}"
                );
                let victim_id = match &elem.data {
                    crate::sequence::SequenceElementData::Interaction {
                        antagonist: Some(antagonist),
                    } => *antagonist,
                    crate::sequence::SequenceElementData::Interaction { antagonist: None } => {
                        panic!(
                            "strangle condolation owner {owner:?} requires an antagonist on {seq_id:?}/{elem_idx}"
                        )
                    }
                    data => panic!(
                        "strangle condolation owner {owner:?} requires Interaction data on {seq_id:?}/{elem_idx}, got {data:?}"
                    ),
                };
                let victim = self.world.entities.get(victim_id).unwrap_or_else(|| {
                    panic!(
                        "strangle condolation owner {owner:?} requires missing victim {victim_id:?}"
                    )
                });
                victim.npc_data().unwrap_or_else(|| {
                    panic!(
                        "strangle condolation victim {victim_id:?} requires NPC data for owner {owner:?}"
                    )
                });
                victim.ai_controller().unwrap_or_else(|| {
                    panic!(
                        "strangle condolation victim {victim_id:?} requires AI state for owner {owner:?}"
                    )
                });

                // Low-priority Wait element to re-park the victim's AI
                // in the default loop.
                self.actor_wait(victim_id);
                #[cfg(test)]
                observe_strangle_condolation_step("Wait");

                // Release the AILOCK_FREEZE acquired in `begin_strangle`.
                self.world
                    .entities
                    .get_mut(victim_id)
                    .expect("validated strangle condolation victim disappeared before unlock")
                    .ai_controller_mut()
                    .expect("validated strangle condolation victim lost AI before unlock")
                    .non_script_unlock(crate::ai::AiLockFlags::FREEZE);
                #[cfg(test)]
                observe_strangle_condolation_step("Unlock");
                let stim = crate::ai::Stimulus::with_human(
                    crate::ai::StimulusType::EventGotHit,
                    owner.index(),
                );
                self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                    sim, victim_id, assets, stim,
                );
                // `RHArtificialMalignity::ThinkUnexpectedEvent`'s EVENT_GOTHIT
                // arm ends in `mpMe->SetViewStatus(EYES_DIE_OR_GET_UNCONSCIOUS)`
                // (`original-code/RHartificialmalignity.cpp:6654`), which the
                // Original applies inside `Think` — i.e. before the next
                // statement here. The port routes that write through the AI
                // recovery outbox, so it has to be drained now; otherwise the
                // gaze reset below compares against a stale eye status and
                // `SetViewStatus` computes the wrong `bTransition`
                // (`original-code/RHelementactornpc.cpp:1375-1379`), freezing
                // a half-finished head-turn angle in the view cone.
                self.tick_ai_pending_resurrection_and_eyes_for_npc(victim_id);
                #[cfg(test)]
                observe_strangle_condolation_step("EventGotHit");

                // Reset the victim's gaze (cosmetic; dead-on-corpse on
                // the kill path, observable on abort).
                let npc = self
                    .world
                    .entities
                    .get_mut(victim_id)
                    .expect("validated strangle condolation victim disappeared before gaze reset")
                    .npc_data_mut()
                    .expect(
                        "validated strangle condolation victim lost NPC data before gaze reset",
                    );
                crate::ai_vision::set_view_status(npc, crate::element::EyeStatus::LookForward);
                #[cfg(test)]
                observe_strangle_condolation_step("LookForward");

                tracing::debug!(
                    pc = owner.index(),
                    victim = victim_id.index(),
                    "send_condolation_card_pc: strangle terminated → wait + unlock(freeze) + event_got_hit + look_forward"
                );
            }

            Command::TakeCorpse => {
                // If mid-grab cancel (`pc.carried.is_some()` but the
                // carried's posture isn't yet `Carried`), drop the
                // partially-grabbed corpse instantly.
                let (carried_id, posture_is_carried) = {
                    let Some(carrier) = self.world.entities.get(owner) else {
                        return;
                    };
                    let Some(carried_id) = carrier.pc_data().and_then(|pc| pc.carried) else {
                        return;
                    };
                    let posture_is_carried = self
                        .world
                        .entities
                        .get(carried_id)
                        .map(|e| e.element_data().posture == crate::element::Posture::Carried)
                        .unwrap_or(false);
                    (carried_id, posture_is_carried)
                };

                if posture_is_carried {
                    // Pickup completed cleanly — nothing to drop.
                    return;
                }

                // Instantaneous drop (already ported as
                // `force_drop_carried_corpse_instant` in
                // `engine/melee.rs`).  Also clear the actor's
                // `active_ability` so a stale Carry slot can't drive a
                // bogus `CarryDone` after the element is gone.
                //
                // `DropCorpse(12, true)` immediately calls the body's
                // priority-WAIT `Wait()`, so close only the synchronous
                // launch/Instruct work created by the drop.  It does *not*
                // call the body's Hourglass: `NewMove` and the first
                // BeingTied/BeingUnconscious `Execute` remain owned by that
                // body's next ordinary creation-order slot.  Eagerly running
                // either here erases the pre-drop old position and makes a
                // body whose slot already passed start its new sprite one
                // frame early (RHelementactorpc.cpp:6502-6506;
                // RHelementactor.cpp:1078-1085).
                let preexisting_sequence_work = self
                    .orders
                    .sequence_manager
                    .take_pending_synchronous_actions();
                self.force_drop_carried_corpse_instant(owner);
                self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                    .unwrap_or_else(|error| {
                        panic!(
                            "TakeCorpse condolation for {owner:?} failed to instruct dropped body {carried_id:?}: {error:?}"
                        )
                    });
                self.orders
                    .sequence_manager
                    .restore_pending_synchronous_actions(preexisting_sequence_work);
                if let Some(carrier) = self.world.entities.get_mut(owner)
                    && let Some(actor) = carrier.actor_data_mut()
                    && actor.active_ability.kind == Some(crate::movement::AbilityKind::Carry)
                {
                    actor.active_ability.clear();
                }

                tracing::debug!(
                    pc = owner.index(),
                    carried = carried_id.index(),
                    "send_condolation_card_pc: take_corpse mid-grab cancel → instant drop"
                );
            }

            _ => {}
        }
    }

    /// Orchestrate a door-battle after `EnemyInHouseAlert`: pick the
    /// nearest unlocked building door, layout defender/attacker
    /// positions around it, and fan out
    /// [`EngineInner::send_before_door_to_fight`] to each occupant.
    ///
    /// `fleeing` is the outnumbered side (leaves the house first);
    /// `pursuing` is the stronger side (follows and provokes for
    /// duel).  The `pursuing.len() >= fleeing.len()` invariant is
    /// enforced by the caller.
    pub(crate) fn init_battle_before_door(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        door_indices: &[u32],
        fleeing: &[EntityId],
        pursuing: &[EntityId],
    ) {
        if fleeing.is_empty() || pursuing.is_empty() {
            return;
        }
        // Use the first fleeing guy as the distance anchor for the
        // nearest-door search.
        let first_fleeing = fleeing[0];
        let Some(entity) = self.get_entity(first_fleeing) else {
            return;
        };
        let first_pos = entity.element_data().position_map();
        let move_box = *entity.position_iface().get_move_box();

        // Pick the unlocked door nearest to first_pos by MaxNorm of
        // (door.point_in - first_pos).
        let (point_in, point_out, point_mid, out_layer, out_sector) = {
            if self.scripts.mission.is_none() {
                return;
            }
            let mut best_idx = None;
            let mut minimum_distance = u16::MAX;
            for &di in door_indices {
                let Some(door) = self.script_domains.interactables.doors.get(di as usize) else {
                    continue;
                };
                if door.is_locked_pc() || door.is_locked_npc_villain() {
                    continue;
                }
                let distance = crate::ai::legacy_nearest_door_distance(
                    door.point_in.x - first_pos.x,
                    door.point_in.y - first_pos.y,
                    false,
                    false,
                );
                if distance < minimum_distance {
                    best_idx = Some(di);
                    minimum_distance = distance;
                }
            }
            let Some(best_idx) = best_idx else {
                return;
            };
            let Some(door) = self
                .script_domains
                .interactables
                .doors
                .get(best_idx as usize)
            else {
                return;
            };
            (
                door.point_in,
                door.point_out,
                door.point_mid,
                door.layer_out,
                door.sector_out,
            )
        };
        let _ = point_in;

        // Battle center = door.point_out; facing = sector(point_out -
        // point_mid) via `vector_to_sector_0_to_15_iso` on the door
        // vector.
        let center = point_out;
        let dir_vec = point_out - point_mid;
        let base_direction =
            crate::position_interface::vector_to_sector_0_to_15_iso(dir_vec.x, dir_vec.y);

        // pursuing >= fleeing is asserted by the caller.
        let num_pursuing = pursuing.len();
        let num_fleeing = fleeing.len();
        debug_assert!(num_pursuing >= num_fleeing);

        for i in 0..num_pursuing {
            // Try 10 random dispersion vectors.  The dispersed
            // direction is `(base + rand(0..7) - 3) & 15`, and the
            // vector magnitude is `30 + rand(0..64)`.  The defender
            // goes to `center + vec`; the attacker to
            // `center + 0.5 * vec`.
            let mut found = false;
            let mut dispersed_direction: i16 = base_direction;
            let mut defender = center;
            let mut attacker = center;
            for _ in 0..10 {
                let jitter =
                    crate::sim_rng::i32(sim, crate::sim_rng::RngSite::DoorFightDispersion, 0..7)
                        - 3;
                let dd = (base_direction + jitter as i16).rem_euclid(16);
                let magnitude = 30.0
                    + crate::sim_rng::u32(sim, crate::sim_rng::RngSite::DoorFightDispersion, 0..64)
                        as f32;
                // Apply the isometric aspect ratio to the Y component
                // — `direction_vector_16` returns a pure unit vector,
                // but the door-battle dispersion expects an
                // iso-compressed offset.
                let (dx, dy_raw) = crate::element_kinds::direction_vector_16(dd);
                let dy = dy_raw * crate::position_interface::ASPECT_RATIO;
                let offset = MapVec::new(dx * magnitude, dy * magnitude);
                let cand = center + offset;
                if self
                    .world
                    .fast_grid
                    .is_straight_movement_authorized(center, cand, out_layer, &move_box)
                {
                    dispersed_direction = dd;
                    defender = cand;
                    attacker = center + offset.scale(0.5);
                    found = true;
                    break;
                }
            }
            if !found {
                // Emergency fallback: collapse both positions to the
                // battle center.
                dispersed_direction = base_direction;
                defender = center;
                attacker = center;
            }

            let defender_pos = Position {
                x: defender.x,
                y: defender.y,
                sector: crate::position_interface::SectorHandle::new(u16::from(out_sector)),
                level: out_layer,
            };
            let attacker_pos = Position {
                x: attacker.x,
                y: attacker.y,
                sector: crate::position_interface::SectorHandle::new(u16::from(out_sector)),
                level: out_layer,
            };

            if i < num_fleeing {
                // Fleeing guy exits and faces back
                // (`dispersed_direction ^ 8`).  Pursuer follows and
                // provokes.
                self.send_before_door_to_fight(
                    sim,
                    assets,
                    fleeing[i],
                    defender_pos,
                    (dispersed_direction ^ 8) as u16,
                    (i as u16) * 10 + 10,
                    None,
                );
                self.send_before_door_to_fight(
                    sim,
                    assets,
                    pursuing[i],
                    attacker_pos,
                    dispersed_direction as u16,
                    ((i + num_fleeing) as u16) * 10 + 20,
                    Some(fleeing[i]),
                );
            } else {
                // Extra pursuers pick a random fleeing guy as their
                // duel target.
                let target_idx = crate::sim_rng::u32(
                    sim,
                    crate::sim_rng::RngSite::DoorFightTarget,
                    0..num_fleeing as u32,
                ) as usize;
                self.send_before_door_to_fight(
                    sim,
                    assets,
                    pursuing[i],
                    attacker_pos,
                    dispersed_direction as u16,
                    ((i + num_fleeing) as u16) * 10 + 20,
                    Some(fleeing[target_idx]),
                );
            }
        }
    }

    /// Send a human actor before a door to fight.  Dispatches on the
    /// actor's concrete type:
    ///
    /// * **Soldier** — builds a `DoorCombatInfo` and synchronously calls the
    ///   soldier's AI with `EventDoorCombat`, entering
    ///   `Substate::AttackingDoorFightDelay` before this fanout continues.
    /// * **PC** — synthesises a four-element sequence (WaitTimer →
    ///   Move → Turn → optional EnterSwordfight) and launches it
    ///   directly.  PCs aren't driven by an enemy-AI brain so the
    ///   stimulus path doesn't apply.
    ///
    /// Any other entity kind reaching this helper is a wiring bug.
    pub(crate) fn send_before_door_to_fight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        actor_id: EntityId,
        goal: Position,
        direction: u16,
        delay: u16,
        adversary: Option<EntityId>,
    ) {
        let kind = self
            .world
            .entities
            .get(actor_id)
            .map(|e| (e.is_pc(), e.is_soldier()));
        match kind {
            Some((true, _)) => {
                self.send_before_door_to_fight_pc(sim, actor_id, goal, direction, delay, adversary);
            }
            Some((_, true)) => {
                let info = DoorCombatInfo {
                    delay,
                    goal,
                    direction,
                    adversary: adversary.map(|id| id.index()).unwrap_or(0),
                };
                // Original Soldier::SendBeforeDoorToFight calls
                // Think(EVENT_DOOR_COMBAT) directly. The state/target/timer
                // update must settle before later same-frame callbacks (such
                // as the completed PassDoor's EventReachPoint) are delivered.
                self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                    sim,
                    actor_id,
                    assets,
                    Stimulus::with_door_combat(StimulusType::EventDoorCombat, info),
                );
            }
            _ => {
                tracing::warn!(
                    actor = actor_id.index(),
                    "send_before_door_to_fight: actor is neither PC nor Soldier"
                );
            }
        }
    }

    /// PC override of [`Self::send_before_door_to_fight`].  Builds a
    /// synthetic `WaitTimer → Move → Turn → [EnterSwordfight]`
    /// sequence and launches it on the PC.
    ///
    /// Cross-sector moves use the shared gate-routing builder with
    /// this method's wait element as a prefix and the turn / optional
    /// swordfight elements as a tail.
    ///
    /// **Enemy-VIP filter:** the swordfight gate is
    /// `enemy.is_some() && !(!is_robin && enemy.is_vip)` — i.e.
    /// non-Robin PCs refuse to engage VIP enemies.  Soldiers use the
    /// cached enemy-AI VIP flag; civilians use the cached civilian type.
    fn send_before_door_to_fight_pc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        pc_id: EntityId,
        goal: Position,
        direction: u16,
        delay: u16,
        enemy_to_attack: Option<EntityId>,
    ) {
        use crate::sequence::{Field, FieldValue, MoveFlags, Sequence, SequenceElementData};

        let is_robin = self
            .world
            .entities
            .get(pc_id)
            .and_then(|e| e.pc_data())
            .map(|pc| pc.robin)
            .unwrap_or(false);

        // `WaitTimer` element with `Field::Timer = delay`.
        let mut wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(pc_id));
        wait.set_property(Field::Timer, FieldValue::Integer(delay as u32));

        let mut tail_elements: Vec<SequenceElement> = Vec::new();
        let mut turn = SequenceElement::new_generic(1, Command::Turn, Some(pc_id));
        turn.set_property(Field::Direction, FieldValue::Integer(direction as u32));
        tail_elements.push(turn);

        // Optional `EnterSwordfight`, gated by
        // `enemy_to_attack.is_some() && (is_robin || !enemy_is_vip)`.
        if let Some(enemy_id) = enemy_to_attack {
            let enemy_is_vip = self
                .world
                .entities
                .get(enemy_id)
                .map(door_combat_enemy_is_vip)
                .unwrap_or(false);
            if is_robin || !enemy_is_vip {
                let mut enter =
                    SequenceElement::new_generic(1, Command::EnterSwordfight, Some(pc_id));
                enter.set_property(Field::Opponent, FieldValue::Element(enemy_id));
                // No jumpline for this swordfight (door-battle is sector-local).
                enter.set_property(Field::JumplineDestination, FieldValue::Integer(0));
                tail_elements.push(enter);
            }
        }

        let source = self.get_entity(pc_id).map(|e| {
            let elem = e.element_data();
            let (door_handle, door_direction) = current_door_for_route_source(e);
            (
                elem.position_map(),
                elem.sector(),
                elem.layer(),
                e.actor_auth_info(),
                door_handle,
                door_direction,
            )
        });

        if let Some((
            raw_source_pos,
            Some(raw_source_sector),
            raw_source_layer,
            auth,
            door_handle,
            door_direction,
        )) = source
            && let Some(goal_sector) = goal.sector
        {
            // Original `RHSequence::AppendMoveToSequence` adapts an actor
            // whose `GetDoor()` is non-null onto the committed far side
            // before it compares source and goal sectors. In particular, a
            // PC already leaving this building for the door-fight goal must
            // not construct (and draw the random wait for) a second exit.
            let (source_pos, source_sector, _source_layer) = adapt_source_to_current_door(
                &self.script_domains.interactables.doors,
                door_handle,
                door_direction,
            )
            .map(|(point, sector, layer)| {
                (
                    point,
                    crate::position_interface::SectorHandle::new(sector).unwrap_or_else(|| {
                        panic!("door-fight route for {pc_id:?} adapted to invalid sector {sector}")
                    }),
                    layer,
                )
            })
            .unwrap_or((raw_source_pos, raw_source_sector, raw_source_layer));

            if source_sector != goal_sector {
                let path = {
                    let level = self.world.fast_grid.level.clone();
                    self.scripts.mission.as_ref().and_then(|_| {
                        crate::gate::find_path_gates(
                            &self.script_domains.interactables.doors,
                            (source_pos.x, source_pos.y),
                            source_sector.get(),
                            (goal.x, goal.y),
                            goal_sector.get(),
                            Some(&auth),
                            false,
                            &|sector| self.building_sector_is_authorized(sector),
                            &|sector| {
                                level
                                    .sectors
                                    .iter()
                                    .find(|candidate| candidate.sector_number == sector)
                                    .and_then(|candidate| candidate.lift_type)
                            },
                        )
                    })
                };
                if let Some(path) = path
                    && !path.is_empty()
                {
                    // Attribute only a route that is actually about to enter the
                    // gate builder, where RuntimeBuildingExitWait can be drawn.
                    self.debug_building_exit_wait_pc_route(pc_id, source_sector, goal_sector);
                    let _ = self.build_gate_movement_sequence(
                        sim,
                        pc_id,
                        Some(source_sector),
                        path,
                        GoalShape::Point {
                            point: MapPoint::new(goal.x, goal.y),
                            tolerance: 0.0,
                        },
                        goal.level,
                        crate::order::OrderType::RunningUpright,
                        true,
                        1.0,
                        MoveFlags::empty(),
                        vec![wait],
                        tail_elements,
                        false,
                        false,
                    );
                    return;
                }
            }
        }

        let mut sequence = Sequence::new();
        let mut level: u16 = 1;

        sequence.append_element(wait);
        level += 1;

        // Single-MOVE same-sector port — see method doc for the
        // cross-sector caveat.  Uses the `RunningUpright` animation.
        let mut move_elem = SequenceElement::new_movement(
            level,
            Command::Move,
            Some(pc_id),
            OrderType::RunningUpright,
        );
        if let SequenceElementData::Movement {
            destination,
            layer,
            sector,
            tolerance,
            flags,
            ..
        } = &mut move_elem.data
        {
            *destination = crate::coordinates::MapPoint {
                x: goal.x,
                y: goal.y,
            };
            *layer = goal.level;
            *sector = goal.sector;
            *tolerance = 0.0;
            *flags = MoveFlags::empty();
        }
        sequence.append_element(move_elem);
        level += 1;

        for mut elem in tail_elements {
            elem.command_level = level;
            sequence.append_element(elem);
            level += 1;
        }

        self.launch_sequence(sequence);
    }
}

fn door_combat_enemy_is_vip(entity: &Entity) -> bool {
    match entity {
        Entity::Soldier(s) => s.npc.ai_brain.enemy().map(|en| en.is_vip).unwrap_or(false),
        Entity::Civilian(c) => {
            c.civilian.cached_civilian_type == crate::profiles::CivilianType::Vip
        }
        Entity::Pc(_) => {
            panic!("door combat PC enemy VIP lookup requires LevelAssets profile data")
        }
        _ => panic!("door combat enemy is not a human actor"),
    }
}

/// Pick the slow-vs-very-slow turning speed for an ale-intoxicated soldier.
///
/// Angles 14/15/0/1/2 of `(direction - goal) & 15` (the soldier is
/// nearly aligned with the goal) return `true` (→ `TurnVerySlow`);
/// every other delta returns `false` (→ `TurnSlow`).  `direction`
/// and `goal` are 16-sector compass values (0..=15).
///
/// Wired into [`crate::engine::movement::EngineInner::tick_move`] via the
/// drunken-turn pre-pass: each tick while a drunken soldier has an
/// active walk path, the pre-pass advances the soldier's facing one
/// step toward the movement-vector direction using this function to
/// pick the step size.
pub fn turn_drunken_is_very_slow(direction: u16, goal: u16) -> bool {
    let delta = direction.wrapping_sub(goal) & 15;
    matches!(delta, 14 | 15 | 0 | 1 | 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence::SequenceElementData;

    fn door_fight_route_fixture(triggers_fired: u8) -> (EngineInner, EntityId, Position) {
        use crate::element::{ActiveDoorPass, ActorData, ActorPc, ElementData, HumanData, PcData};
        use crate::engine::MissionScript;
        use crate::fast_find_grid::GridSector;
        use crate::gate::{Door, DoorIndex, DoorType};
        use crate::scb::{ClassEntry, SCB_VERSION, ScbFile};
        use crate::sector::{SectorNumber, SectorType};

        let mission = MissionScript::from_scb(ScbFile {
            version: SCB_VERSION,
            classes: vec![ClassEntry {
                source_file: "door_fight_route_test.scs".into(),
                class_name: "StartUp".into(),
                size_of_member_variables: 0,
                member_variables: Vec::new(),
                functions: Vec::new(),
                quads: Vec::new(),
            }],
        })
        .expect("minimal mission script");

        let building_sector = SectorNumber::new(118);
        let outside_sector = SectorNumber::new(0);
        let point_in = MapPoint::new(1513.0, 777.0);
        let point_out = MapPoint::new(1500.0, 1017.0);
        let goal = Position {
            x: 1490.0,
            y: 1020.0,
            sector: crate::position_interface::SectorHandle::new(0),
            level: 0,
        };

        let mut engine = EngineInner::new();
        engine.scripts.mission = Some(mission);
        engine.script_domains.interactables.doors = vec![Door {
            door_type: DoorType::Building,
            point_in,
            point_out,
            point_mid: MapPoint::new(1506.0, 897.0),
            layer_in: 6,
            layer_out: 0,
            sector_in: building_sector,
            sector_out: outside_sector,
            ..Door::default()
        }];
        crate::gate::build_gate_links(&mut engine.script_domains.interactables.doors);

        let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
        for (sector_number, sector_type, layer) in [
            (building_sector, SectorType::BUILDING, 6),
            (outside_sector, SectorType::MOTION | SectorType::AREA, 0),
        ] {
            let index = level.sectors.len();
            level.sector_number_map.insert(sector_number, index);
            level.sectors.push(GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type,
                layer,
                sector_number,
                door_index: None,
                lift_type: None,
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

        let mut element = ElementData {
            kind: crate::element::ElementKind::ActorPc,
            posture: crate::element::Posture::Upright,
            ..ElementData::default()
        };
        element.set_position_map(point_in);
        element.set_sector(crate::position_interface::SectorHandle::new(118));
        element.set_layer(6);
        let pc = engine.add_entity(Entity::Pc(ActorPc {
            element,
            actor: ActorData {
                active_door_pass: Some(ActiveDoorPass {
                    door_index: DoorIndex(0),
                    direct: false,
                    position_direct: false,
                    steps: Default::default(),
                    triggers_fired,
                    current_action: OrderType::RunningUpright,
                    current_reverse: false,
                    saved_action_state: None,
                }),
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData {
                life_points: 100,
                ..PcData::default()
            },
        }));

        (engine, pc, goal)
    }

    fn door_fight_sequence(engine: &EngineInner, pc: EntityId) -> &crate::sequence::Sequence {
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .find(|sequence| {
                sequence
                    .elements
                    .iter()
                    .any(|element| element.owner == Some(pc))
            })
            .expect("door-fight sequence")
    }

    #[test]
    fn door_fight_route_adapts_selected_pass_door_before_building_exit_rng() {
        use crate::sim_rng::{RngSite, with_draw_trace};

        let sim = crate::sim_rng::test_context();
        let (mut crossing, pc, goal) = door_fight_route_fixture(0);
        let (_, crossing_draws) = with_draw_trace(|| {
            crossing.send_before_door_to_fight_pc(&sim, pc, goal, 4, 20, None);
        });
        assert_eq!(
            crossing_draws,
            Vec::<RngSite>::new(),
            "selected PassDoor commits the route source to the outside goal sector"
        );
        let sequence = door_fight_sequence(&crossing, pc);
        assert_eq!(
            sequence
                .elements
                .iter()
                .map(|element| element.command)
                .collect::<Vec<_>>(),
            [Command::WaitTimer, Command::Move, Command::Turn]
        );
        let SequenceElementData::Movement {
            destination,
            sector,
            ..
        } = &sequence.elements[1].data
        else {
            panic!("door-fight Move changed element kind")
        };
        assert_eq!(*destination, MapPoint::new(goal.x, goal.y));
        assert_eq!(*sector, goal.sector);

        let (mut callback_complete, pc, goal) = door_fight_route_fixture(1);
        let (_, callback_complete_draws) = with_draw_trace(|| {
            callback_complete.send_before_door_to_fight_pc(&sim, pc, goal, 4, 20, None);
        });
        assert_eq!(
            callback_complete_draws,
            [
                RngSite::RuntimeBuildingExitWait,
                RngSite::RuntimeBuildingExitWait
            ],
            "after PassDoor clears GetDoor, the same raw building source must build one exit"
        );
        let sequence = door_fight_sequence(&callback_complete, pc);
        assert!(
            sequence
                .elements
                .iter()
                .any(|element| element.command == Command::ChangePosition),
            "control must actually contain the building-exit route"
        );
    }

    #[test]
    fn map_movement_condolence_uses_unpadded_authored_bbox() {
        let mut grid = crate::fast_find_grid::FastFindGrid::default();
        grid.size_map(2, 2);

        let inside_authored_map = MapPoint::new(64.0, 127.0);
        assert!(grid.is_inside_grid_point(inside_authored_map));
        assert!(grid.level.map_bbox.contains_point(inside_authored_map));

        let inside_padded_grid_only = MapPoint::new(64.0, 128.0);
        assert!(grid.is_inside_grid_point(inside_padded_grid_only));
        assert!(!grid.level.map_bbox.contains_point(inside_padded_grid_only));
    }

    #[test]
    fn turn_drunken_very_slow_window() {
        // Facing close to goal → TurnVerySlow.
        for delta in [14u16, 15, 0, 1, 2] {
            assert!(turn_drunken_is_very_slow(delta, 0));
        }
        // Further off → TurnSlow.
        for delta in [3u16, 5, 8, 10, 13] {
            assert!(!turn_drunken_is_very_slow(delta, 0));
        }
    }

    #[test]
    fn turn_drunken_wraps_around() {
        // (direction - goal) must wrap: direction=0, goal=1 → delta=15 → very slow.
        assert!(turn_drunken_is_very_slow(0, 1));
        // direction=0, goal=3 → delta=13 → slow.
        assert!(!turn_drunken_is_very_slow(0, 3));
    }
}
