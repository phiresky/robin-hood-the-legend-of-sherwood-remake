//! Player command dispatch — applies [`PlayerCommand`]s to the engine.
//!
//! This is the single entry point for all player-initiated sim mutations.
//! The input system resolves raw events into commands by reading engine
//! state immutably; this module executes them.
//!
//! Authorization, replay-batch interpretation, preflight and shared macro capture
//! stay here. Domain handlers execute only after that boundary; moving a handler
//! must not move recording into the handler or eagerly drain its queued sequence.

mod combat;
mod interaction;
mod interaction_route;
mod object_use;
mod posture;
mod quick_actions;
mod seat_lifecycle;
mod selection;

#[cfg(test)]
use combat::legacy_random_input_sword_seek_distance;
pub(crate) use interaction_route::command_action_distance_animation;
#[cfg(test)]
use interaction_route::{interaction_distance, target_interaction_assert_source_sector};
#[cfg(test)]
use object_use::determine_use_command;
pub(super) use object_use::is_pc_takable;
pub use object_use::{coin_pickup_target, object_pickup_command};
#[cfg(test)]
use quick_actions::quick_action_tail_command;

use super::{CameraDisplayState, EngineInner, LevelAssets};
#[cfg(test)]
use super::{HostDisplayState, InputState};
#[cfg(test)]
use crate::coordinates::MapPoint;
use crate::element::{Command, Entity, EntityId};
#[cfg(test)]
use crate::player_command::{CompositeSwordTechnique, GestureQuality};
use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
#[cfg(test)]
use crate::sequence::{
    Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
};
use crate::titbit::QuickAction;
#[cfg(test)]
use crate::titbit::{ElementHandle, INVALID_ID, TitbitKind};

/// Interpretation of adjacency inside one already-resolved command batch.
///
/// Live/runtime batches may contain a recursively forwarded selection action,
/// while Original parity traces contain only independently recorded messages:
/// Nested-selection recording includes raw-mouse depth-2 messages
/// but omits the depth-3 restitution emitted by `SelectPc` itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionCommandBatchMode {
    InferNestedSelection,
    IndependentRecordedMessages,
}

#[inline]
fn group_move_actor_accepts_command(actor: EntityId, recorded_failed_routes: &[EntityId]) -> bool {
    !recorded_failed_routes.contains(&actor)
}

/// Quick-action phases authored by a concrete original-game interaction
/// site rather than by the PC's currently selected portrait action.
///
/// These paths all arrive through interaction recording, but their
/// subsequent titbit additions deliberately choose their own phase:
/// Bow input uses the available-bow hint, while the two PC-on-PC
/// contextual player-character clicks use the walk hint.
/// The latter is true even though the acting PC owns Carry or Jump. Untie is
/// the Rust extension and deliberately records under the existing Tie phase.
fn recorded_interaction_quick_phase(command: Command) -> Option<QuickAction> {
    match command {
        Command::ShootBow => Some(QuickAction::BowOk),
        Command::TakeCorpse | Command::ClimbUpOnShoulders => Some(QuickAction::Walk),
        Command::Untie => Some(QuickAction::Tie),
        _ => None,
    }
}

/// Layer passed to the original game's ground-target marker creation.
///
/// The captured command retains the selected layer because the thrown purse
/// itself needs it, but purse-input processing authors its QA marker on
/// literal layer zero. Wasp is the closely related exception that really
/// does use the selected layer; Net also authors literal zero before capture.
fn recorded_ground_target_titbit_layer(command: Command, captured_layer: u16) -> u16 {
    match command {
        Command::ThrowPurse | Command::ThrowNet | Command::ThrowStone => 0,
        _ => captured_layer,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordedInteractionIdentityError {
    MissingOrNonPcActor,
    MissingTarget,
}

impl EngineInner {
    /// Rebuild every relationship-derived cache after a runtime diplomacy
    /// edit. No stale opponent/detectable may survive a peace treaty, and a
    /// newly hostile faction must become perceivable without reloading.
    fn reconcile_diplomacy_runtime(&mut self) {
        crate::diplomacy::reconcile_entities(
            &mut self.world.entities,
            &self.mission_domain.diplomacy,
        );
    }

    /// Apply one authoritative command batch with an explicit interpretation
    /// of adjacent selection messages.
    pub(crate) fn apply_frame_commands_with_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        commands: &[PlayerInput],
        mode: SelectionCommandBatchMode,
    ) {
        let mut camera = std::mem::take(&mut self.feedback.cutscene_camera.display);
        self.apply_commands_authoritative(sim, &mut camera, assets, commands, mode);
        self.feedback.cutscene_camera.display = camera;
    }

    fn apply_commands_authoritative(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        camera: &mut CameraDisplayState,
        assets: &LevelAssets,
        commands: &[PlayerInput],
        mode: SelectionCommandBatchMode,
    ) {
        for (index, inp) in commands.iter().enumerate() {
            if inp.player_id != crate::player_command::PlayerId::HOST
                && inp.command.requires_host_authority()
            {
                tracing::warn!(
                    player_id = ?inp.player_id,
                    command = ?inp.command,
                    "non-host seat cannot mutate host-authoritative session state"
                );
                continue;
            }
            let seat = self.ensure_seat(inp.player_id);
            if !self.reusable_cloak_command_is_authorized(inp, seat) {
                continue;
            }
            let recorded_nested_selection_action = mode
                == SelectionCommandBatchMode::InferNestedSelection
                && matches!(
                    (&inp.command, commands.get(index + 1)),
                    (
                        PlayerCommand::SelectPc {
                            pc_id: selected,
                            append: false,
                        },
                        Some(PlayerInput {
                            player_id,
                            command:
                                PlayerCommand::SelectResolvedAction { pc_id: nested, .. }
                                | PlayerCommand::CancelAction { pc_id: nested },
                        }),
                    ) if *player_id == inp.player_id && selected == nested
                );
            self.apply_command_for_seat_with_replay_context(
                sim,
                camera,
                assets,
                seat,
                &inp.command,
                recorded_nested_selection_action,
            );
        }
    }

    fn validate_recorded_interaction_identities(
        &self,
        actor: EntityId,
        target: EntityId,
    ) -> Result<(), RecordedInteractionIdentityError> {
        if self
            .get_entity(actor)
            .and_then(|entity| entity.pc_data())
            .is_none()
        {
            return Err(RecordedInteractionIdentityError::MissingOrNonPcActor);
        }
        if self.get_entity(target).is_none() {
            return Err(RecordedInteractionIdentityError::MissingTarget);
        }
        Ok(())
    }

    /// Apply a batch of player commands for the current frame.
    /// Per-frame scroll dedupe (`frame_scrolled`) is reset at the end
    /// of `perform_hourglass` (after `tick_display_state`), not here —
    /// the live game pushes scroll commands via `apply_command`
    /// (singular) one-at-a-time during input handling, while the
    /// rollback path calls `apply_commands` in a batch; both paths
    /// must dedupe identically, and the display-state tick still needs
    /// to see which directions were pressed this frame.
    #[cfg(test)]
    pub(crate) fn apply_commands(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        commands: &[PlayerInput],
    ) {
        self.apply_commands_with_mode(
            sim,
            display,
            input,
            assets,
            commands,
            SelectionCommandBatchMode::InferNestedSelection,
        );
    }

    #[cfg(test)]
    pub(crate) fn apply_commands_with_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        commands: &[PlayerInput],
        mode: SelectionCommandBatchMode,
    ) {
        let event_start = self.feedback.pending_side_effects.host_events.len();
        let mut camera = self.feedback.cutscene_camera.display.clone();
        self.apply_commands_authoritative(sim, &mut camera, assets, commands, mode);
        self.feedback.cutscene_camera.display = camera;
        for event in self.feedback.pending_side_effects.host_events[event_start..]
            .iter()
            .cloned()
        {
            display.apply_host_event(input, event);
        }
    }

    /// Apply a batch of commands tagged as issued by the local seat.
    /// Convenience wrapper around [`Self::apply_commands`] for the
    /// single-player input pipeline: each raw [`PlayerCommand`] is
    /// stamped with [`crate::player_command::PlayerId::HOST`] before
    /// dispatch.  Live multiplayer pipelines should build
    /// [`PlayerInput`]s with their `Host::local_seat` and call
    /// [`Self::apply_commands`] directly so the seat tag is
    /// data-driven.
    #[cfg(test)]
    pub(crate) fn apply_local_commands(
        &mut self,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        commands: &[PlayerCommand],
    ) {
        let sim = self.control.simulation_context();
        let commands = commands
            .iter()
            .cloned()
            .map(PlayerInput::host)
            .collect::<Vec<_>>();
        self.apply_commands(&sim, display, input, assets, &commands);
    }

    /// Apply a single [`PlayerCommand`] as if it came from
    /// [`crate::player_command::PlayerId::HOST`].
    ///
    /// Test adapter over the same authoritative batch dispatcher used by
    /// [`crate::engine::Engine::advance_frame`].
    #[cfg(test)]
    pub(crate) fn apply_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        cmd: &PlayerCommand,
    ) {
        self.apply_commands(
            sim,
            display,
            input,
            assets,
            &[PlayerInput::host(cmd.clone())],
        );
    }

    fn apply_command_for_seat_with_replay_context(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        seat: usize,
        cmd: &PlayerCommand,
        recorded_nested_selection_action: bool,
    ) {
        use PlayerCommand::*;

        // A new manual order cannot inherit an earlier quick-action feat.
        match cmd {
            LaunchInteraction { actor, .. }
            | LaunchGroundTarget { actor, .. }
            | LaunchSelfAbility { actor, .. }
            | LaunchScrollRead { actor, .. }
            | EnterSwordfight { actor, .. }
            | SwordStrikeCmd { actor, .. } => {
                self.mission_domain
                    .achievements
                    .cancel_quick_action_for_manual_order(*actor);
            }
            StopPc { pc_id } => {
                self.mission_domain
                    .achievements
                    .cancel_quick_action_for_manual_order(*pc_id);
            }
            GroupMove { actors, .. } => {
                for actor in actors {
                    self.mission_domain
                        .achievements
                        .cancel_quick_action_for_manual_order(*actor);
                }
            }
            _ => {}
        }

        // Pre-flight reachability gate for object Take clicks. Bail
        // early when no authorized position can be found from the movement box,
        // target position, and target layer—silently skipping *both* the macro-side
        // sequence registration and the live launch. We gate here,
        // before `record_macro_step_for` (which would otherwise append a
        // `QuickActionStep`) and before the `LaunchInteraction` arm
        // (which installs the QA titbit and kicks off
        // `apply_interaction_with_seek`).
        if let LaunchInteraction {
            actor,
            target,
            command: Command::Take,
            ..
        } = cmd
            && self.is_object_take_target(*target)
            && !self.object_take_reachable(*actor, *target)
        {
            return;
        }

        // A recorded interaction is about to mutate the actor's QA slot in
        // `record_macro_step_for`. Original already holds concrete actor and
        // antagonist pointers at this boundary; missing replay identities are
        // therefore invalid state, not a NoAction/default-position command.
        if let LaunchInteraction { actor, target, .. } = cmd
            && self.players.qa_recording_for.contains(actor)
        {
            match self.validate_recorded_interaction_identities(*actor, *target) {
                Ok(()) => {}
                Err(RecordedInteractionIdentityError::MissingOrNonPcActor) => {
                    panic!("recorded interaction owner {actor:?} is missing or is not a PC")
                }
                Err(RecordedInteractionIdentityError::MissingTarget) => {
                    panic!("recorded interaction target {target:?} is missing")
                }
            }
        }

        // The original game resolves and authorizes the complete ale-seeking movement before
        // deciding whether to store it as a QA. A forbidden sector or failed
        // move-box authorization therefore stores nothing and leaves macro
        // recording armed; do not let the shared hook append a fake step or
        // the dispatch arm send STOP_RECORDING_MACRO in that case.
        if let DropAleAt {
            actor,
            target_pos,
            already_authorized,
            goal_override,
            goal_sector_index_override,
            ..
        } = cmd
            && self.players.qa_recording_for.contains(actor)
            && self
                .resolve_drop_ale_target(
                    *actor,
                    *target_pos,
                    *already_authorized,
                    *goal_override,
                    *goal_sector_index_override,
                )
                .is_none()
        {
            return;
        }

        if let Err(error) = cmd.validate_sword_gesture(
            self.control.sim_config.more_combat_gestures,
            self.control.sim_config.gesture_quality_damage,
        ) {
            tracing::warn!(seat, %error, "rejecting invalid sword-gesture command");
            return;
        }

        // Append-while-recording hook.  Records one `QuickActionStep`
        // per sim-affecting player command addressed at the currently
        // recording PC, keyed by the resolved Action (portrait bar)
        // so the macro-icon strip can render per-step titbit frames.
        self.record_macro_step_for(seat, cmd, assets);
        match cmd {
            Noop => {} // consumed input, no action
            ScriptKeyPressed { virtual_key } => {
                self.call_script_vm(
                    sim,
                    assets,
                    crate::engine::ScriptVmKey::Global,
                    "KeyPressed",
                    &[*virtual_key],
                    crate::natives::ScriptCallFrame::default(),
                )
                .unwrap_or_else(|error| panic!("Script KeyPressed failed: {error}"));
            }

            // ── Movement ────────────────────────────────────────
            GroupMove {
                actors,
                destination,
                running,
                show_marker,
                goal_override,
                goal_sector_index_override,
                door_route_override,
                recorded_gate_routes,
                recorded_failed_gate_routes,
            } => {
                self.perform_group_move(
                    sim,
                    assets,
                    actors,
                    *destination,
                    *running,
                    *show_marker,
                    *goal_override,
                    *goal_sector_index_override,
                    *door_route_override,
                    recorded_gate_routes,
                    recorded_failed_gate_routes,
                );
                // Play command-acceptance speech for the PC
                // that just accepted the move — the "yes, milord" bark.
                // It lives outside `perform_group_move` because the engine
                // helper has no access to `LevelAssets`; this is the
                // command-dispatch entry point where the assets are in
                // scope.
                for &pc_id in actors {
                    if self.players.qa_recording_for.contains(&pc_id) {
                        continue;
                    }
                    if group_move_actor_accepts_command(pc_id, recorded_failed_gate_routes) {
                        self.hero_speaking(
                            assets,
                            pc_id,
                            crate::engine::melee::HERO_ACCEPT_COMMAND,
                        );
                    }
                }
            }
            StopPc { pc_id } => {
                // Stopping an actor leaves its default Wait
                // element alone. For real movement it rewrites/stops the
                // sequence so its transition can finish; it does not
                // directly force the action state to Waiting.
                self.stop_owner(*pc_id, crate::sequence::SequencePriority::Normal);
            }

            // ── Sequence-based interactions ──────────────────────
            LaunchInteraction {
                actor,
                target,
                command,
                running,
            } => {
                self.dispatch_target_interaction(sim, assets, actor, target, command, running);
            }
            LaunchGroundTarget {
                actor,
                target_pos,
                command,
                target_field,
                titbit_layer,
            } => {
                self.dispatch_ground_target(actor, target_pos, command, target_field, titbit_layer);
            }
            LaunchSelfAbility { actor, command } => {
                self.dispatch_self_ability(assets, actor, command);
            }
            LaunchScrollRead {
                actor,
                target,
                running,
            } => {
                self.dispatch_scroll_read(sim, actor, target, running);
            }

            // ── Swordfight ──────────────────────────────────────
            EnterSwordfight {
                actor,
                target,
                running,
            } => {
                self.apply_enter_swordfight(sim, assets, *actor, *target, *running);
            }
            SwordStrikeCmd {
                actor,
                target,
                command,
                composite,
                gesture_quality,
                with_seek,
                seek_distance,
            } => {
                self.dispatch_player_sword_strike(
                    assets,
                    actor,
                    target,
                    command,
                    composite,
                    gesture_quality,
                    with_seek,
                    seek_distance,
                );
            }
            SetPrincipalOpponent { actor, opponent_id } => {
                self.set_as_new_principal_opponent(assets, *actor, *opponent_id);
            }

            // ── Action bar ──────────────────────────────────────
            SelectAction {
                pc_id,
                action_index,
            } => {
                let selected_before = self.players.seats[seat].selection.clone();
                if self.select_pc_action_by_index_from_message(
                    assets,
                    seat,
                    *pc_id,
                    *action_index as u8,
                ) {
                    self.close_player_select_action_stop_callbacks(sim, assets, selected_before);
                }
            }
            SelectResolvedAction { pc_id, action } => {
                let selected_before = self.players.seats[seat].selection.clone();
                self.set_pc_action_from_message(assets, seat, *pc_id, *action);
                self.close_player_select_action_stop_callbacks(sim, assets, selected_before);
            }
            SelectPlannedAction { pc_id, action } => {
                if !self.players.seats[seat].selection.contains(pc_id) {
                    tracing::warn!(?pc_id, ?action, "ignored planned action for unselected PC");
                    return;
                }
                self.players.seats[seat].planned_action =
                    if self.players.seats[seat].planned_action == *action {
                        crate::profiles::Action::NoAction
                    } else {
                        *action
                    };
                self.players.seats[seat].planned_shield_target = None;
            }
            SelectPlannedShieldProtected {
                actor,
                protected_pc,
            } => {
                let planned = self.players.seats[seat].planned_action;
                if !self.players.seats[seat].selection.contains(actor)
                    || !matches!(
                        planned,
                        crate::profiles::Action::Shield | crate::profiles::Action::BigShield
                    )
                {
                    tracing::warn!(
                        ?actor,
                        ?protected_pc,
                        ?planned,
                        "ignored invalid planned shield protectee"
                    );
                    return;
                }
                let valid = self
                    .get_entity(*protected_pc)
                    .and_then(crate::element::Entity::pc_data)
                    .is_some_and(|pc| pc.life_points > 0);
                if !valid {
                    tracing::warn!(
                        ?protected_pc,
                        "ignored unavailable planned shield protectee"
                    );
                    return;
                }
                self.players.seats[seat].planned_shield_target = Some((*actor, *protected_pc));
            }
            CancelPlannedAction => {
                self.players.seats[seat].planned_action = crate::profiles::Action::NoAction;
                self.players.seats[seat].planned_shield_target = None;
            }
            CancelAction { pc_id } => {
                self.set_pc_action_from_message(
                    assets,
                    seat,
                    *pc_id,
                    crate::profiles::Action::NoAction,
                );
            }
            UnselectAllActions => {
                for pc_id in self.players.seats[seat].selection.clone() {
                    self.unselect_action(pc_id);
                }
                self.players.seats[seat].selected_action = crate::profiles::Action::NoAction;
            }
            MouseRightDown => {
                self.feedback
                    .pending_side_effects
                    .host_events
                    .push(super::HostEvent::SetRightMouseDown { down: true });
            }
            MouseRightUp => {
                self.feedback
                    .pending_side_effects
                    .host_events
                    .push(super::HostEvent::SetRightMouseDown { down: false });
            }
            ClearShootList { pc_id } => {
                // Clear the retained human-instruction FIFO. Keep the
                // broader pending-element cleanup for pre-instruction work that
                // has not reached that FIFO yet.
                self.clear_pc_shoot_list(*pc_id);
                let resolver = Self::priority_resolver(&self.world.entities);
                self.orders.sequence_manager.stop_pending_elements_matching(
                    *pc_id,
                    Command::ShootBow,
                    crate::sequence::SequencePriority::Preference,
                    &resolver,
                );
            }
            DropAmmo {
                pc_id,
                action_id,
                amount,
            } => {
                self.dispatch_drop_ammo(pc_id, action_id, amount);
            }
            DropAleAt {
                actor,
                target_pos,
                running,
                already_authorized,
                goal_override,
                goal_sector_index_override,
                recorded_gate_path,
            } => {
                self.dispatch_drop_ale(
                    actor,
                    target_pos,
                    running,
                    already_authorized,
                    goal_override,
                    goal_sector_index_override,
                    recorded_gate_path,
                );
            }
            ShieldSelectProtected {
                actor: _,
                protected_pc,
            } => {
                // Stash the focused PC as the shield protectee and
                // flip `is_protected = false` so the next click resolves
                // the danger point.  No sequence is launched.
                self.world.shield.protected_pc = Some(*protected_pc);
                self.world.shield.is_protected = false;
            }
            RaiseShieldWithDanger {
                actor,
                protected_pc,
                danger_point,
                danger_point_layer,
            } => {
                self.dispatch_player_raise_shield(
                    actor,
                    protected_pc,
                    danger_point,
                    danger_point_layer,
                );
            }

            // ── Posture ─────────────────────────────────────────
            CrouchDown => self.apply_crouch_down(sim, seat),
            StandUp => self.apply_stand_up(sim, seat),

            // ── Selection ───────────────────────────────────────
            SelectPc { pc_id, append } => {
                self.dispatch_pc_selection(
                    assets,
                    seat,
                    pc_id,
                    append,
                    recorded_nested_selection_action,
                );
            }
            TogglePcSelection { pc_id } => {
                self.toggle_pc_selection(assets, seat, *pc_id);
                self.update_recording_after_selection_change();
            }
            UnselectPc { pc_id } => {
                if self.players.seats[seat].selection.contains(pc_id) {
                    self.unselect_single_pc(*pc_id);
                    self.update_recording_after_selection_change();
                    self.emit_character_selection_followups();
                }
            }
            BoxSelect { pt1, pt2, shift } => {
                self.apply_box_select(assets, seat, *pt1, *pt2, *shift);
                self.update_recording_after_selection_change();
            }
            BoxUnselect { pt1, pt2 } => {
                self.apply_box_unselect(seat, *pt1, *pt2);
                self.update_recording_after_selection_change();
            }
            SelectAllPcs => {
                self.select_all_pcs(assets, seat);
                self.update_recording_after_selection_change();
            }
            UnselectAllPcs => {
                self.unselect_all_pcs(seat);
                self.update_recording_after_selection_change();
            }
            AssignQuickGroup { index } => {
                self.assign_quick_group(seat, *index as usize);
            }
            RecallQuickGroup { index } => {
                self.recall_quick_group(assets, seat, *index as usize);
                self.update_recording_after_selection_change();
            }
            SelectByPortrait {
                portrait_index,
                append,
            } => {
                self.dispatch_portrait_selection(assets, seat, portrait_index, append);
            }
            SelectTacticalUnits { soldiers, append } => {
                self.select_tactical_units(seat, soldiers, *append);
            }
            BoxSelectTacticalUnits { pt1, pt2, shift } => {
                self.box_select_tactical_units(seat, *pt1, *pt2, *shift);
            }
            ClearTacticalSelection => {
                self.players.tactical.ensure_seat(seat).selection.clear();
            }
            PinTacticalSelection => self.pin_tactical_selection(seat),
            UnpinTacticalGroup { group_id } => self.unpin_tactical_group(seat, *group_id),
            SelectTacticalGroup { group_id, append } => {
                if !append {
                    self.unselect_all_pcs(seat);
                }
                self.select_tactical_group(seat, *group_id, *append);
            }
            PageTacticalPortraits { delta } => self.page_tactical_portraits(seat, *delta),
            MoveTacticalUnits {
                soldiers,
                destination,
                running,
                formation,
            } => {
                let leaders = self.players.seats[seat].selection.clone();
                self.command_tactical_move(
                    sim,
                    assets,
                    soldiers,
                    &leaders,
                    *destination,
                    *running,
                    *formation,
                )
            }
            SetCombatStance { soldiers, stance } => {
                self.set_tactical_stance(soldiers, *stance);
            }
            SetTacticalFormation {
                soldiers,
                formation,
            } => self.set_tactical_formation(soldiers, *formation),
            SetTacticalPatrol {
                soldiers,
                destination,
                formation,
            } => self.set_tactical_patrol(sim, assets, soldiers, *destination, *formation),
            SetTacticalFollow {
                soldiers,
                hero,
                formation,
            } => self.set_tactical_follow(assets, soldiers, *hero, *formation),
            ReleaseTacticalControl => self.release_tactical_control(),

            // ── Special ─────────────────────────────────────────
            ResetComa { pc_id } => self.reset_coma(assets, *pc_id),
            SendReinforcement { pc_id } => self.request_reinforcement(*pc_id),
            // Use actor-level fast-movement conversion so the pathfinder + queued
            // transitions get rewritten, not just the element-level
            // action.
            MakePcFast { pc_id } => self.actor_make_fast(sim, *pc_id),
            BeggarDontTalkStamp { beggar_id } => self.stamp_beggar_dont_talk_counter(*beggar_id),
            MakePcSlow { pc_id } => self.actor_make_slow(sim, *pc_id),
            MakePcUpright { pc_id } => self.actor_make_upright(sim, *pc_id),
            MakePcCrouched { pc_id } => self.actor_make_crouched(sim, *pc_id),

            ChangeState(req) => {
                self.change_state(display, seat, *req);
            }

            // ── Speed / pacing ──────────────────────────────────
            SetFastForward => {
                self.set_fast_forward();
            }

            // ── QA macro recording ─────────────────────────────
            StopRecordingMacro => {
                self.stop_recording_macro();
            }
            StartMacro { pc, slot } => {
                self.apply_start_macro(sim, display, assets, *pc, *slot);
            }
            DeleteMacro { pc, slot } => {
                self.apply_delete_macro(display, *pc, *slot);
            }
            StartRecordingMacro { pc, slot } => {
                self.apply_start_recording_macro(seat, *pc, *slot);
            }
            ChangeQaMemory { slot } => {
                self.apply_change_qa_memory(seat, *slot);
            }
            QueueQuickAction { action, command } => {
                let command = command.to_player_command();
                self.apply_queue_quick_action(sim, display, assets, seat, *action, &command);
            }
            MakeQueuedActionFast { pc_id } => {
                self.apply_make_queued_action_fast(sim, *pc_id);
            }
            SetLockAlt(on) => {
                self.players.seats[seat].is_lock_alt = *on;
            }
            KeyControl => {
                self.players.seats[seat].action_before_control =
                    self.players.seats[seat].selected_action;
                self.save_action_for_selected_pcs(seat);
                // Park every selected PC at NoAction so the held ctrl
                // key lets the follow-up move command run unobstructed.
                // The per-PC `current_action` write + `unselect_action`
                // loop matches the body of `set_pc_action` for the
                // NoAction path, skipping the rubber-band /
                // `ignore_next_drag` side-effects (those belong to the
                // action-pick flow, not a modifier key).
                for id in self.players.seats[seat].selection.clone() {
                    let cur = self
                        .get_entity(id)
                        .and_then(|e| e.pc_data())
                        .map(|pc| pc.current_action)
                        .unwrap_or(crate::profiles::Action::NoAction);
                    if cur != crate::profiles::Action::NoAction {
                        self.unselect_action(id);
                    }
                    if let Some(entity) = self.get_entity_mut(id)
                        && let Some(pc) = entity.pc_data_mut()
                    {
                        pc.current_action = crate::profiles::Action::NoAction;
                    }
                }
                self.feedback
                    .pending_side_effects
                    .invalidate_trajectory_preview = true;
                self.players.seats[seat].selected_action = crate::profiles::Action::NoAction;
            }
            #[cfg(not(target_os = "macos"))]
            KeyReleaseControl => {
                // Original restores the messenger-global action captured on
                // Ctrl press, then fans that one action over the selection.
                let restore = self.players.seats[seat].action_before_control;
                let ids = self.players.seats[seat].selection.clone();
                for id in ids {
                    let cur = match self.get_entity(id).and_then(|e| e.pc_data()) {
                        Some(pc) => pc.current_action,
                        None => continue,
                    };
                    if cur != restore {
                        self.unselect_action(id);
                    }
                    if let Some(entity) = self.get_entity_mut(id)
                        && let Some(pc) = entity.pc_data_mut()
                    {
                        pc.current_action = restore;
                    }
                }
                self.players.seats[seat].selected_action = restore;
                self.feedback
                    .pending_side_effects
                    .invalidate_trajectory_preview = true;
            }
            #[cfg(target_os = "macos")]
            KeyReleaseControl => {
                // macOS uses ctrl as stop-action, so releasing ctrl
                // does NOT restore the pre-ctrl action.  No-op.
            }

            // ── Per-frame aim orientation ──────────────────────
            PerformOrientation { mouse_map } => {
                self.perform_orientation(assets, *mouse_map);
            }
            PerformResolvedOrientation {
                pc_id,
                action,
                mouse_map,
                target,
            } => {
                // The Original emits an orientation record only while
                // the messenger action is this action. That global UI
                // state is not otherwise present in every trace frame, so
                // the authoritative replay command must reconstruct it
                // before the pre-update mission script can query
                // HasAnyActionSelected.
                self.players.seats[seat].selected_action = *action;
                self.perform_resolved_orientation(assets, *pc_id, *action, *mouse_map, *target);
            }

            // ── Cheats ──────────────────────────────────────────
            SetGoldenEyeMode { on } => {
                self.set_golden_eye_mode(*on);
            }

            // ── Host-driven sim mutations routed through commands ─
            SetMenToBlazonConversionMode { on } => {
                self.set_men_to_blazon_conversion_mode(*on);
            }
            RegisterPeasantName { name } => {
                self.register_peasant_name(name.clone());
            }
            DispatchStartupMessage { msg, arg1, arg2 } => {
                self.dispatch_startup_message(sim, assets, *msg, *arg1, *arg2);
            }
            RevealAllBlips => {
                self.reveal_all_blips();
            }
            CampaignSelectNextMission { mission_idx } => {
                if let Some(campaign) = Some(&mut self.mission_domain.campaign) {
                    campaign.select_next_mission(*mission_idx, &assets.profile_manager);
                }
            }
            CampaignSwapPendingToAccessibleMissions => {
                if let Some(campaign) = Some(&mut self.mission_domain.campaign) {
                    campaign.swap_pending_to_accessible_missions();
                }
            }
            CampaignHarvestProductionSectorState => {
                self.harvest_production_sector_state(assets);
            }
            CampaignSellProductionItem {
                request_id,
                prod_type,
                quantity,
            } => {
                self.sell_sherwood_production_item(
                    assets,
                    seat,
                    *request_id,
                    *prod_type,
                    *quantity,
                );
            }
            CampaignConvertSelectedPeasantsToBlazons => {
                self.convert_selected_peasants_to_blazons(sim, &assets.profile_manager);
            }
            ApplyQuitMissionUpdates {
                exit_code,
                difficulty,
                completed_at_unix_seconds,
                campaign_run_nonce,
            } => {
                self.apply_quit_mission_updates(
                    sim,
                    assets,
                    *exit_code,
                    *difficulty,
                    *completed_at_unix_seconds,
                    *campaign_run_nonce,
                );
            }
            QuitMissionRequested => {
                // The flag to set depends on whether the mission is
                // already won.  The tick's mission-end arms at
                // `tick.rs:354-368` consume these flags next frame.
                if self.mission_domain.state.mission_won {
                    self.mission_domain.state.quit_won = true;
                } else {
                    self.mission_domain.state.quit_interrupted = true;
                }
            }
            TeleportSelectedToPoint {
                dest,
                layer,
                sector,
            } => {
                self.manage_input_process_teleport(*dest, *layer, *sector);
            }

            // ── Minimap ─────────────────────────────────────────
            MinimapResize { base, corner_size } => {
                let screen = Self::director_camera_view_size();
                self.feedback
                    .pending_side_effects
                    .host_events
                    .push(super::HostEvent::Minimap(super::MinimapHostEvent::Resize {
                        base: *base,
                        corner_size: *corner_size,
                        screen_width: screen.x,
                        screen_height: screen.y,
                    }));
            }
            MinimapMouseDown {
                click_pt,
                continuing_drag,
            } => {
                let screen = Self::director_camera_view_size();
                self.feedback
                    .pending_side_effects
                    .host_events
                    .push(super::HostEvent::Minimap(
                        super::MinimapHostEvent::MouseDown {
                            click_pt: *click_pt,
                            screen_width: screen.x,
                            screen_height: screen.y,
                        },
                    ));
                // The host resolves this before dispatch. Do not infer an
                // engine message from rollback-local minimap scratch.
                if *continuing_drag {
                    self.orders.messenger.send(crate::messenger::Message::new(
                        crate::messenger::MessageType::Simple(
                            crate::messenger::SimpleMessage::UiHasFocus,
                        ),
                    ));
                    self.feedback
                        .pending_side_effects
                        .host_events
                        .push(super::HostEvent::ClearInputFocus);
                }
            }
            MinimapMouseMove {
                mouse_pt,
                left_mouse_down,
                continuing_drag,
            } => {
                let screen = Self::director_camera_view_size();
                self.feedback
                    .pending_side_effects
                    .host_events
                    .push(super::HostEvent::Minimap(
                        super::MinimapHostEvent::MouseMove {
                            mouse_pt: *mouse_pt,
                            left_mouse_down: *left_mouse_down,
                            screen_width: screen.x,
                            screen_height: screen.y,
                        },
                    ));
                if *continuing_drag {
                    // Continuing-drag focus is command-derived. The host
                    // presentation mutation above may legitimately differ
                    // on a rollback scratch display.
                    self.orders.messenger.send(crate::messenger::Message::new(
                        crate::messenger::MessageType::Simple(
                            crate::messenger::SimpleMessage::UiHasFocus,
                        ),
                    ));
                    self.feedback
                        .pending_side_effects
                        .host_events
                        .push(super::HostEvent::ClearInputFocus);
                }
            }
            MinimapMouseUp { on_minimap } => {
                self.feedback
                    .pending_side_effects
                    .host_events
                    .push(super::HostEvent::Minimap(
                        super::MinimapHostEvent::MouseUp {
                            on_minimap: *on_minimap,
                        },
                    ));
            }
            CenterCameraOnPoint { point } => {
                let level_size = self.feedback.cutscene_camera.level_size;
                assert!(
                    point.x.is_finite()
                        && point.y.is_finite()
                        && point.x >= 0.0
                        && point.y >= 0.0
                        && point.x <= level_size.x
                        && point.y <= level_size.y,
                    "camera center point ({}, {}) is outside required level bounds ({}, {})",
                    point.x,
                    point.y,
                    level_size.x,
                    level_size.y
                );
                if self.is_zoom_possible() {
                    self.players.seats[seat].locker_active = false;
                    self.center_on_point(seat, *point);
                }
            }
            MinimapRightClick => {
                // Unconditional close animation start (no
                // transition_counter guard) so a right-click during the
                // opening animation immediately reverses to closing.
                self.feedback
                    .pending_side_effects
                    .host_events
                    .push(super::HostEvent::Minimap(
                        super::MinimapHostEvent::RightClick,
                    ));
            }
            MinimapToggle => {
                // Open if hidden, close if shown.  Both arms set the
                // counters unconditionally so an in-flight transition
                // reverses immediately, and the close arm also flips
                // the UI state to Selected.
                self.feedback
                    .pending_side_effects
                    .host_events
                    .push(super::HostEvent::Minimap(super::MinimapHostEvent::Toggle));
            }

            // ── Display / UI setters ────────────────────────────
            SelectFollowElement { entity_id } => {
                self.select_follow_element(seat, *entity_id);
            }
            ClearNpcDoubleStatusBarFlags => {
                self.clear_npc_double_status_bar_flags();
            }
            SetAmountOfSpeaking { amount } => {
                assert!(
                    *amount <= 9,
                    "SetAmountOfSpeaking requires the sound-menu range 0..=9, got {amount}"
                );
                self.control.sim_config.amount_of_speaking = *amount;
            }
            SetFixHardReactionTimes { enabled } => {
                self.control.sim_config.fix_hard_reaction_times = *enabled;
            }
            SetFogOfWar { enabled } => {
                if seat != usize::from(PlayerId::HOST.0) {
                    tracing::warn!(seat, "ignored non-host fog-of-war setting command");
                } else if self.control.rng.original_replay_cursor().is_some() && *enabled {
                    tracing::warn!("Original parity playback cannot enable fog of war");
                    self.control.sim_config.fog_of_war = false;
                } else {
                    self.control.sim_config.fog_of_war = *enabled;
                    if *enabled {
                        // Settings commands are applied after the ordinary frame
                        // tick. Populate current sight immediately so enabling
                        // fog cannot present one black or stale frame while
                        // waiting for the next periodic scan.
                        self.refresh_fog_of_war(assets, true);
                    }
                }
            }
            SetTimedMissionsEnabled { enabled } => {
                if seat == usize::from(crate::player_command::PlayerId::HOST.0) {
                    self.control.sim_config.enable_timed_missions = *enabled;
                } else {
                    tracing::warn!(seat, "ignored non-host timed-mission setting command");
                }
            }
            SetDynamicAmbienceEnabled { enabled } => {
                if seat == usize::from(crate::player_command::PlayerId::HOST.0) {
                    self.control.sim_config.enable_dynamic_ambience = *enabled;
                } else {
                    tracing::warn!(seat, "ignored non-host ambience setting command");
                }
            }
            SetCombatGestureRules {
                more_combat_gestures,
                gesture_quality_damage,
            } => {
                if seat == usize::from(PlayerId::HOST.0) {
                    self.control.sim_config.more_combat_gestures = *more_combat_gestures;
                    self.control.sim_config.gesture_quality_damage = *gesture_quality_damage;
                } else {
                    tracing::warn!(seat, "ignored non-host combat-gesture setting command");
                }
            }
            SetUnbindingEnabled { enabled } => {
                self.control.sim_config.enable_unbinding = *enabled;
            }
            SetCleanHandsNpcKillsInvalidate { enabled } => {
                self.control.sim_config.clean_hands_npc_kills_invalidate = *enabled;
                self.mission_domain
                    .achievements
                    .refresh_clean_hands_rule(*enabled)
                    .expect("achievement results changed after mission finalization");
            }
            SetReusableCloaks { enabled } => {
                self.set_reusable_cloaks_enabled(*enabled);
            }
            SetItemGameplayConfig { config } => {
                if self.control.rng.original_replay_cursor().is_some() {
                    tracing::warn!("ignoring item-rebalance command during Original-parity replay");
                    self.control.sim_config.item_gameplay =
                        crate::gameplay_config::ItemGameplayConfig::classic();
                } else {
                    self.control.sim_config.item_gameplay = *config;
                }
                let reliable_ale = self
                    .control
                    .sim_config
                    .item_gameplay
                    .ale_reliable_distraction;
                for (actor_id, entity) in self.world.entities.actors_mut() {
                    // Ale completion resolves a SoldierProfile from SoldierData;
                    // autonomous PC enemies must not gain the soldier-only
                    // zero-beer eligibility without that completion contract.
                    let reliable_for_actor = if !reliable_ale {
                        false
                    } else if let Entity::Soldier(soldier) = entity {
                        !assets
                            .profile_manager
                            .get_soldier(soldier.soldier.soldier_profile_index)
                            .unwrap_or_else(|| {
                                panic!(
                                    "ale reliability requires missing soldier profile {:?} for {actor_id:?}",
                                    soldier.soldier.soldier_profile_index,
                                )
                            })
                            .vip
                    } else {
                        false
                    };
                    if let Some(enemy) = entity.enemy_ai_mut() {
                        enemy.ale_reliable_distraction = reliable_for_actor;
                    }
                }
            }
            SetNoiseDistractionFeedback { enabled } => {
                if self.control.rng.original_replay_cursor().is_some() {
                    tracing::warn!("ignoring stone-feedback command during Original-parity replay");
                    self.control.sim_config.noise_distraction_feedback = false;
                } else {
                    self.control.sim_config.noise_distraction_feedback = *enabled;
                }
            }
            SetSherwoodTrading { enabled } => {
                if seat == usize::from(PlayerId::HOST.0) {
                    self.control.sim_config.sherwood_trading = *enabled;
                } else {
                    tracing::warn!(seat, "non-host Sherwood trading setting command rejected");
                }
            }
            SetDiplomacyEnabled { enabled } => {
                self.control.sim_config.diplomacy = *enabled;
                self.mission_domain.diplomacy.set_enabled(*enabled);
                self.reconcile_diplomacy_runtime();
                self.refresh_fog_of_war(assets, true);
            }
            SetNpcFactionWars { enabled } => {
                self.control.sim_config.npc_faction_wars = *enabled;
                self.mission_domain.diplomacy.set_npc_faction_wars(*enabled);
                self.reconcile_diplomacy_runtime();
                self.refresh_fog_of_war(assets, true);
            }
            SetDiplomacyRelationship {
                first,
                second,
                relationship,
            } => {
                self.mission_domain
                    .diplomacy
                    .set_relationship_ids(*first, *second, *relationship)
                    .unwrap_or_else(|error| panic!("invalid diplomacy command: {error}"));
                self.reconcile_diplomacy_runtime();
                self.refresh_fog_of_war(assets, true);
            }
            HeroSpeak { pc_id, expression } => {
                self.hero_speaking(assets, *pc_id, *expression);
            }

            // Host-side record of a drained modal. The actual
            // dismissal happens in the game session loop; the engine
            // has no state to mutate for this variant — carrying it in
            // the command stream is what lets replays auto-dismiss.
            ModalDismiss { .. } => {}

            // ── Seat lifecycle ──────────────────────────────────
            // The target seat is in the command payload, NOT the
            // dispatch `seat` parameter — the host can issue these
            // on behalf of a peer that hasn't materialised yet.
            ConnectSeat {
                player_id: target,
                nickname,
            } => {
                self.dispatch_connect_seat(target, nickname);
            }
            DisconnectSeat { player_id: target } => {
                self.dispatch_disconnect_seat(target);
            }
        }
    }

    fn apply_command_authoritative(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        camera: &mut CameraDisplayState,
        assets: &LevelAssets,
        seat: usize,
        command: &PlayerCommand,
    ) {
        self.apply_command_for_seat_with_replay_context(sim, camera, assets, seat, command, false);
    }

    /// Close the actor-stop callbacks authored by a player action selection
    /// before the next engine actor walk.
    ///
    /// The original game stops every selected PC synchronously. A
    /// stopped `TakeCorpse` can therefore run its condolence immediately:
    /// `DropCorpse(12, true)` releases the body and calls the body's `Wait()`
    /// before creation-order actor slots begin. Rust queues cards
    /// to avoid re-entrant borrows, so leaving them for the actor/global drain
    /// makes a body whose slot has not yet run miss its first idle Execute.
    /// Registered action-entry elements remain on the manager FIFO; this
    /// closes only selected-owner condolence cards created by the synchronous
    /// Stop stack.
    fn close_player_select_action_stop_callbacks(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        selected_before: Vec<EntityId>,
    ) {
        for owner in selected_before {
            self.dispatch_condolations_for_owner_boundary(sim, owner, assets);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        ActorCivilian, ActorData, ActorPc, ActorSoldier, Camp, ElementBonus, ElementData,
        ElementKind, ElementNet, ElementProjectile, ElementScroll, ElementTarget, Entity, FxData,
        HumanData, NetData, NpcData, ObjectData, ObjectType, PcData, Posture, ProjectileData,
        SoldierData, TargetData,
    };
    use crate::engine::MissionScript;
    use crate::engine::ScrollStatus;
    use crate::macro_store::{QaReplayCommand, QuickActionStep};
    use crate::profiles::{Action, CharacterProfile, ProfileManager};
    use crate::sprite::Sprite;
    use crate::sprite_script::{SpriteScript, UNMAPPED};

    #[test]
    fn campaign_mutations_are_host_authoritative() {
        let (mut engine, assets, _pc) = setup_pc_engine(&[]);
        let sim = crate::sim_rng::test_context();
        assert!(!engine.is_men_to_blazon_conversion_mode());

        engine.apply_frame_commands_with_mode(
            &sim,
            &assets,
            &[PlayerInput::new(
                crate::player_command::PlayerId(1),
                PlayerCommand::SetMenToBlazonConversionMode { on: true },
            )],
            SelectionCommandBatchMode::InferNestedSelection,
        );
        assert!(
            !engine.is_men_to_blazon_conversion_mode(),
            "a client seat must not mutate campaign UI state"
        );

        engine.apply_frame_commands_with_mode(
            &sim,
            &assets,
            &[PlayerInput::new(
                crate::player_command::PlayerId::HOST,
                PlayerCommand::SetMenToBlazonConversionMode { on: true },
            )],
            SelectionCommandBatchMode::InferNestedSelection,
        );
        assert!(engine.is_men_to_blazon_conversion_mode());
    }

    #[test]
    fn deterministic_settings_and_seat_lifecycle_are_host_authoritative() {
        let (mut engine, assets, _pc) = setup_pc_engine(&[]);
        let sim = crate::sim_rng::test_context();
        let rebalanced_items = crate::gameplay_config::ItemGameplayConfig::default();
        assert_ne!(engine.control.sim_config.item_gameplay, rebalanced_items);
        assert!(engine.control.sim_config.noise_distraction_feedback);

        engine.apply_frame_commands_with_mode(
            &sim,
            &assets,
            &[
                PlayerInput::new(
                    crate::player_command::PlayerId(1),
                    PlayerCommand::SetItemGameplayConfig {
                        config: rebalanced_items,
                    },
                ),
                PlayerInput::new(
                    crate::player_command::PlayerId(1),
                    PlayerCommand::SetNoiseDistractionFeedback { enabled: false },
                ),
                PlayerInput::new(
                    crate::player_command::PlayerId(1),
                    PlayerCommand::ConnectSeat {
                        player_id: crate::player_command::PlayerId(7),
                        nickname: "forged".to_string(),
                    },
                ),
            ],
            SelectionCommandBatchMode::InferNestedSelection,
        );
        assert_ne!(engine.control.sim_config.item_gameplay, rebalanced_items);
        assert!(engine.control.sim_config.noise_distraction_feedback);
        assert!(engine.players.seats.get(7).is_none());

        engine.apply_frame_commands_with_mode(
            &sim,
            &assets,
            &[
                PlayerInput::host(PlayerCommand::SetItemGameplayConfig {
                    config: rebalanced_items,
                }),
                PlayerInput::host(PlayerCommand::SetNoiseDistractionFeedback { enabled: false }),
                PlayerInput::host(PlayerCommand::ConnectSeat {
                    player_id: crate::player_command::PlayerId(7),
                    nickname: "authenticated".to_string(),
                }),
            ],
            SelectionCommandBatchMode::InferNestedSelection,
        );
        assert_eq!(engine.control.sim_config.item_gameplay, rebalanced_items);
        assert!(!engine.control.sim_config.noise_distraction_feedback);
        assert_eq!(engine.players.seats[7].nickname, "authenticated");
    }

    #[test]
    fn ale_reliability_command_updates_spawned_soldiers_but_not_autonomous_pcs() {
        let sim_context = crate::sim_rng::test_context();
        let mut engine = EngineInner::new();
        let mut assets = LevelAssets::new();
        std::sync::Arc::make_mut(&mut assets.profile_manager)
            .soldiers
            .extend([
                crate::profiles::SoldierProfile::default(),
                crate::profiles::SoldierProfile {
                    vip: true,
                    ..Default::default()
                },
            ]);
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        let soldier_id = engine.add_entity(Entity::Soldier(ActorSoldier {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                ai: crate::element::AiActorData {
                    ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                    ..Default::default()
                },
                ..Default::default()
            },
            soldier: SoldierData::default(),
        }));
        let vip_soldier_id = engine.add_entity(Entity::Soldier(ActorSoldier {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                ai: crate::element::AiActorData {
                    ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                    ..Default::default()
                },
                ..Default::default()
            },
            soldier: SoldierData {
                soldier_profile_index: crate::profiles::SoldierProfileIdx(1),
                ..Default::default()
            },
        }));
        let autonomous_pc_id = engine.add_entity(Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorPc;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                ai: Some(Box::new(crate::element::AiActorData {
                    ai_brain: crate::element::AiBrain::Enemy(Box::default()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        }));

        let mut rules = crate::gameplay_config::ItemGameplayConfig::classic();
        rules.ale_reliable_distraction = true;
        engine.apply_command(
            &sim_context,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::SetItemGameplayConfig { config: rules },
        );

        assert!(
            engine
                .get_entity(soldier_id)
                .and_then(Entity::enemy_ai)
                .expect("test soldier enemy AI")
                .ale_reliable_distraction
        );
        assert!(
            !engine
                .get_entity(vip_soldier_id)
                .and_then(Entity::enemy_ai)
                .expect("test VIP soldier enemy AI")
                .ale_reliable_distraction
        );
        assert!(
            !engine
                .get_entity(autonomous_pc_id)
                .and_then(Entity::enemy_ai)
                .expect("test autonomous PC enemy AI")
                .ale_reliable_distraction
        );
    }

    #[test]
    fn recorded_failed_group_move_does_not_emit_accept_bark() {
        let failed = EntityId::Pc(crate::entity_id::PcId(136));
        let succeeded = EntityId::Pc(crate::entity_id::PcId(137));
        assert!(!group_move_actor_accepts_command(failed, &[failed]));
        assert!(group_move_actor_accepts_command(succeeded, &[failed]));
    }

    #[test]
    fn target_interaction_door_adaptation_omits_redundant_sector_assertion() {
        use crate::coordinates::{MapBBox, MapPoint};
        use crate::fast_find_grid::GridSector;
        use crate::sector::{SectorNumber, SectorType};

        let target_sector = crate::position_interface::SectorHandle::new(51).unwrap();

        assert_eq!(
            target_interaction_assert_source_sector(
                crate::position_interface::SectorHandle::new(51).unwrap(),
                target_sector,
            ),
            None,
            "adapting door 10 from sector 48 onto its sector-51 far side must produce the Original's direct interaction Move"
        );
        assert_eq!(
            target_interaction_assert_source_sector(
                crate::position_interface::SectorHandle::new(48).unwrap(),
                target_sector,
            ),
            crate::position_interface::SectorHandle::new(48),
            "a genuinely distinct adapted source must retain the movement sequence's leading AssertPosition"
        );

        let source_index = crate::fast_find_grid::SectorIndex::new(17).unwrap();
        let target_index = crate::fast_find_grid::SectorIndex::new(18).unwrap();
        let exact_source = target_sector.with_arena_index(source_index);
        let exact_target = target_sector.with_arena_index(target_index);
        assert_eq!(
            target_interaction_assert_source_sector(exact_source, exact_target),
            Some(exact_source),
            "overlapping public sector numbers are distinct original-game sector identities"
        );

        let mut engine = EngineInner::new();
        let recovered_index = engine.world.fast_grid_mut().add_sector(
            GridSector {
                points: vec![
                    MapPoint::new(0.0, 0.0),
                    MapPoint::new(400.0, 0.0),
                    MapPoint::new(400.0, 400.0),
                    MapPoint::new(0.0, 400.0),
                ],
                bounding_box: MapBBox::from_coords(0.0, 0.0, 400.0, 400.0),
                sector_type: SectorType::MOTION | SectorType::AREA | SectorType::BUILDING,
                layer: 0,
                sector_number: SectorNumber::new(51),
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
            },
            0,
        );
        let mut actor = ElementData::default();
        actor.set_position_map(MapPoint::new(100.0, 100.0));
        actor.set_sector(Some(target_sector));
        let recovered_source = crate::engine::ai::ai_view_position_sector(&engine, &actor)
            .expect("number-only actor sector is recoverable from its position");
        let exact_target = target_sector.with_arena_index(
            crate::fast_find_grid::SectorIndex::new(recovered_index)
                .expect("test arena index is valid"),
        );
        assert_eq!(
            recovered_source, exact_target,
            "loaded actors that retained only a public number recover the exact sector identity"
        );
        assert_eq!(
            target_interaction_assert_source_sector(recovered_source, exact_target),
            None,
            "same-sector target interactions must emit Move directly instead of AssertPosition then losing Move at the building tail"
        );
    }

    /// Build an `(engine, assets, pc_id)` triple with a single PC
    /// whose character profile carries the supplied `(action, max_ammo)`
    /// pairs.  The PC's live ammo counts start at 0 — so storage-left
    /// equals `max_ammo` for every configured action, which is what
    /// the pickup-eligibility tests want.
    fn setup_pc_engine(actions: &[(Action, u16)]) -> (EngineInner, LevelAssets, EntityId) {
        let mut actions_arr = [Action::NoAction; crate::profiles::NUMBER_OF_PC_ACTIONS];
        let mut max_ammo_arr = [0u16; crate::profiles::NUMBER_OF_PC_ACTIONS];
        for (i, (a, m)) in actions.iter().enumerate() {
            actions_arr[i] = *a;
            max_ammo_arr[i] = *m;
        }
        let profile = CharacterProfile {
            actions: actions_arr,
            action_max_ammo: max_ammo_arr,
            ..CharacterProfile::default()
        };

        let mut pm = ProfileManager::new();
        pm.characters.push(profile);
        let mut assets = LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(pm);

        let mut engine = EngineInner::new();

        // Campaign with one `PcDescription` referencing the profile
        // at index 0.  Default ammo is 0 → full storage.
        let mut campaign = crate::campaign::Campaign::default();
        campaign.characters.push(crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            instanced: true,
            ..Default::default()
        });
        engine.mission_domain.campaign = campaign;

        let pc_id = engine.add_entity(Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                profile_index: crate::profiles::CharacterProfileIdx(0),
                campaign_description_index: Some(0),
                life_points: 50,
                ..PcData::default()
            },
        }));
        engine
            .get_entity_mut(pc_id)
            .unwrap()
            .position_iface_mut()
            .set_pathfinder_index(crate::position_interface::PathfinderIndex::new(0).unwrap());

        (engine, assets, pc_id)
    }

    #[test]
    fn mixed_domain_dispatch_preserves_sequence_registration_order() {
        let (mut engine, assets, actor) = setup_pc_engine(&[]);
        let target = spawn_pc_at(&mut engine, 90.0, 10.0);
        let commands = [
            PlayerCommand::DropAmmo {
                pc_id: actor,
                action_id: 3,
                amount: 7,
            },
            PlayerCommand::SwordStrikeCmd {
                actor,
                target,
                command: Command::SwordstrikeThrustA,
                composite: None,
                gesture_quality: GestureQuality::PERFECT,
                with_seek: false,
                seek_distance: None,
            },
            PlayerCommand::LaunchSelfAbility {
                actor,
                command: Command::LeaveBeggar,
            },
        ];
        engine.apply_commands(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &commands
                .into_iter()
                .map(PlayerInput::host)
                .collect::<Vec<_>>(),
        );

        let elements: Vec<_> = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| &sequence.elements)
            .collect();
        assert_eq!(
            elements
                .iter()
                .map(|element| element.command)
                .collect::<Vec<_>>(),
            [
                Command::DropAmmo,
                Command::SwordstrikeThrustA,
                Command::LeaveBeggar
            ],
            "cross-domain execution registers sequences in input order without draining them"
        );
        assert!(elements.iter().all(|element| element.owner == Some(actor)));
        assert!(matches!(
            elements[0].get_property(Field::ActionId),
            Some(FieldValue::Integer(3))
        ));
        assert!(matches!(
            elements[0].get_property(Field::Amount),
            Some(FieldValue::Integer(7))
        ));
    }

    #[test]
    fn self_ability_domain_records_once_before_stopping_without_live_launch() {
        let (mut engine, assets, actor) = setup_pc_engine(&[(Action::Whistle, 0)]);
        engine.players.seats[0].selection.push(actor);
        engine
            .get_entity_mut(actor)
            .and_then(Entity::pc_data_mut)
            .unwrap()
            .current_action = Action::Whistle;
        let sim = crate::sim_rng::test_context();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(actor),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchSelfAbility {
                actor,
                command: Command::WhistleCmd,
            },
        );
        assert!(!engine.is_recording_macro());
        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
        let slot = engine
            .players
            .macro_store
            .get(actor)
            .unwrap()
            .slot(0)
            .unwrap();
        assert_eq!(slot.steps.len(), 1);
        assert_eq!(slot.steps[0].action, Action::Whistle);
        assert_eq!(
            slot.steps[0].replay,
            QaReplayCommand::SelfAbility {
                command: Command::WhistleCmd
            }
        );
    }

    #[test]
    fn manual_shield_quick_action_records_without_live_launch_and_replays_exact_route() {
        let (mut engine, assets, actor) = setup_pc_engine(&[(Action::Shield, 0)]);
        let protected_pc = spawn_pc_at(&mut engine, 80.0, 30.0);
        engine.players.seats[0].selection.push(actor);
        engine
            .get_entity_mut(actor)
            .and_then(Entity::pc_data_mut)
            .expect("shield actor is a PC")
            .current_action = Action::Shield;

        let sim = crate::sim_rng::test_context();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(actor),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::ShieldSelectProtected {
                actor,
                protected_pc,
            },
        );
        assert!(engine.is_recording_macro());
        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);

        let danger_point = WorldPoint3D::new(140.0, 215.0, 35.0);
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::RaiseShieldWithDanger {
                actor,
                protected_pc,
                danger_point,
                danger_point_layer: 7,
            },
        );

        assert!(!engine.is_recording_macro());
        assert_eq!(
            engine.orders.sequence_manager.sequence_count(),
            0,
            "recording stores the shield sequence instead of launching it live"
        );
        let state = engine
            .players
            .macro_store
            .get(actor)
            .expect("recorded shield QA state");
        let slot = state.slot(0).expect("recorded shield QA slot");
        assert_eq!(slot.steps.len(), 1);
        assert_eq!(slot.steps[0].action, Action::Shield);
        assert_eq!(slot.steps[0].position, danger_point.to_map());
        assert_eq!(
            slot.steps[0].replay,
            QaReplayCommand::ShieldRaise {
                protected_pc,
                danger_point,
                danger_point_layer: 7,
            }
        );
        let titbit = engine
            .feedback
            .titbit_manager
            .titbits()
            .iter()
            .find(|titbit| titbit.kind == TitbitKind::QuickAction)
            .expect("recorded shield QA titbit");
        assert_eq!(titbit.phase, crate::titbit::QuickAction::Shield as u16);
        assert_eq!(
            titbit.element_supplier,
            Some(ElementHandle(protected_pc.index()))
        );
        assert_eq!(titbit.element_manager, Some(ElementHandle(actor.index())));
        assert_eq!(titbit.position, danger_point);
        assert_eq!(titbit.layer, 7);

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartMacro {
                pc: Some(actor),
                slot: 0,
            },
        );

        assert!(!engine.has_quick_action(actor, 0));
        assert_eq!(engine.world.shield.danger_point, danger_point);
        assert_eq!(engine.world.shield.danger_point_layer, 7);
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("shield QA launches one sequence");
        let seek = sequence.get(0).expect("shield QA begins with Seek");
        assert_eq!(seek.command, Command::Seek);
        let SequenceElementData::Movement {
            element,
            tolerance,
            flags,
            post_seek_sequence,
            ..
        } = &seek.data
        else {
            panic!("shield QA must begin with a movement element");
        };
        assert_eq!(*element, Some(protected_pc));
        assert_eq!(*tolerance, 50.0);
        assert!(flags.contains(crate::sequence::MoveFlags::SEEK_SHIELD));
        let raise = post_seek_sequence
            .as_ref()
            .and_then(|post_seek| post_seek.get(0))
            .expect("shield QA Seek owns RaiseShield continuation");
        assert_eq!(raise.command, Command::RaiseShield);
        assert!(matches!(
            raise.get_property(Field::ShieldDangerPoint),
            Some(FieldValue::Point3D { x, y, z })
                if *x == danger_point.x && *y == danger_point.y && *z == danger_point.z
        ));
        assert!(matches!(
            raise.get_property(Field::ShieldDangerPointLayer),
            Some(FieldValue::Integer(7))
        ));
        assert!(matches!(
            raise.get_property(Field::ShieldProtected),
            Some(FieldValue::Element(id)) if *id == protected_pc
        ));
    }

    #[test]
    fn planned_action_selection_does_not_touch_live_pc_or_launch_work() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Bow, 4)]);
        engine.players.seats[0].selection.push(pc_id);
        let sequence_count = engine.orders.sequence_manager.sequence_count();

        engine.apply_command(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::SelectPlannedAction {
                pc_id,
                action: Action::Bow,
            },
        );

        assert_eq!(engine.players.seats[0].planned_action, Action::Bow);
        assert_eq!(
            engine
                .get_entity(pc_id)
                .and_then(Entity::pc_data)
                .expect("test PC")
                .current_action,
            Action::NoAction
        );
        assert_eq!(
            engine.orders.sequence_manager.sequence_count(),
            sequence_count
        );

        engine.apply_command(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::SelectPlannedAction {
                pc_id,
                action: Action::Bow,
            },
        );
        assert_eq!(engine.players.seats[0].planned_action, Action::NoAction);

        engine.apply_command(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::SelectPlannedAction {
                pc_id,
                action: Action::Bow,
            },
        );
        engine.apply_command(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::CancelPlannedAction,
        );
        assert_eq!(engine.players.seats[0].planned_action, Action::NoAction);
        assert_eq!(
            engine
                .get_entity(pc_id)
                .and_then(Entity::pc_data)
                .expect("test PC")
                .current_action,
            Action::NoAction
        );
    }

    #[test]
    fn planned_shield_first_click_is_per_seat_hashed_state_and_cancel_clears_it() {
        let (mut engine, assets, actor) = setup_pc_engine(&[(Action::Shield, 0)]);
        let protected = spawn_pc_at(&mut engine, 80.0, 30.0);
        engine.players.seats[0].selection.push(actor);
        let sim = crate::sim_rng::test_context();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::SelectPlannedAction {
                pc_id: actor,
                action: Action::Shield,
            },
        );
        let before = robin_util::state_hash::compute(&engine.players.seats[0]);
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::SelectPlannedShieldProtected {
                actor,
                protected_pc: protected,
            },
        );
        assert_eq!(
            engine.planned_shield_protected_for_seat(crate::player_command::PlayerId::HOST, actor),
            Some(protected)
        );
        assert_ne!(
            before,
            robin_util::state_hash::compute(&engine.players.seats[0])
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::CancelPlannedAction,
        );
        assert_eq!(
            engine.planned_shield_protected_for_seat(crate::player_command::PlayerId::HOST, actor),
            None
        );
    }

    #[test]
    fn occupied_manual_recording_stays_live_until_first_capture_and_cancel_preserves_icon() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Whistle, 1)]);
        engine.players.seats[0].selection.push(pc_id);
        let sim = crate::sim_rng::test_context();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchSelfAbility {
                actor: pc_id,
                command: Command::WhistleCmd,
            },
        );
        let original_state = engine
            .players
            .macro_store
            .get(pc_id)
            .expect("captured manual QA")
            .clone();
        let original_titbit = original_state
            .get_slot_titbit(0)
            .expect("captured manual QA titbit");
        let original_icon = engine
            .get_entity(pc_id)
            .and_then(Entity::pc_data)
            .expect("test PC")
            .portrait
            .quick_icons[0];

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        assert!(engine.has_quick_action(pc_id, 0));
        assert_eq!(
            engine
                .players
                .macro_store
                .get(pc_id)
                .and_then(|state| state.get_slot_titbit(0)),
            Some(original_titbit)
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StopRecordingMacro,
        );
        assert_eq!(
            engine.players.macro_store.get(pc_id),
            Some(&original_state),
            "canceling an armed occupied slot must preserve its QA"
        );
        let canceled_icon = engine
            .get_entity(pc_id)
            .and_then(Entity::pc_data)
            .expect("test PC")
            .portrait
            .quick_icons[0];
        assert_eq!(canceled_icon.titbit_id, original_icon.titbit_id);
        assert_eq!(canceled_icon.running, original_icon.running);

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchSelfAbility {
                actor: pc_id,
                command: Command::EnterListen,
            },
        );

        let replacement = engine
            .players
            .macro_store
            .get(pc_id)
            .expect("replacement manual QA");
        assert_eq!(replacement.slot(0).expect("slot zero").steps.len(), 1);
        assert!(matches!(
            replacement.slot(0).expect("slot zero").steps[0].replay,
            QaReplayCommand::SelfAbility {
                command: Command::EnterListen
            }
        ));
        assert_ne!(replacement.get_slot_titbit(0), Some(original_titbit));
        assert!(
            engine
                .feedback
                .titbit_manager
                .titbits()
                .iter()
                .all(|titbit| titbit.id != original_titbit),
            "the first replacement append must retire the former titbit atomically"
        );
    }

    #[test]
    fn manual_sword_strike_executes_without_recording_a_quick_action() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Hit, 1)]);
        let target = spawn_pc_at(&mut engine, 90.0, 10.0);
        let sim = crate::sim_rng::test_context();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::SwordStrikeCmd {
                actor: pc_id,
                target,
                command: Command::SwordstrikeThrustA,
                composite: None,
                gesture_quality: GestureQuality::PERFECT,
                with_seek: false,
                seek_distance: Some(0.0),
            },
        );

        assert!(
            engine.players.qa_recording_for.contains(&pc_id),
            "a live strike does not finish or otherwise mutate macro recording"
        );
        assert!(!engine.has_quick_action(pc_id, 0));
        assert!(engine.feedback.titbit_manager.titbits().is_empty());
        assert!(
            engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(pc_id, |command| {
                    command == Command::SwordstrikeThrustA
                }),
            "the manual strike still executes while macro recording is armed"
        );
    }

    #[test]
    fn legacy_random_sword_seek_reconstructs_original_click_and_gesture_distances() {
        let (mut engine, mut assets, pc_id) = setup_pc_engine(&[]);
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles.characters[0].hth_weapon_id = 1;
        let mut weapon = crate::profiles::HtHWeaponProfile::default();
        weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] = 70;
        weapon.thrusts[crate::weapons::SwordStrike::A as usize].maximal_distance = 60;
        weapon.thrusts[crate::weapons::SwordStrike::B as usize].maximal_distance = 80;
        profiles.hth_weapons.push(weapon);

        assert_eq!(
            legacy_random_input_sword_seek_distance(
                &profiles.hth_weapons[0],
                Command::SwordstrikeThrustA,
            ),
            63.0,
        );
        assert_eq!(
            legacy_random_input_sword_seek_distance(
                &profiles.hth_weapons[0],
                Command::SwordstrikeThrustB,
            ),
            72.0,
        );

        let target = spawn_pc_at(&mut engine, 90.0, 10.0);
        engine.apply_command(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::SwordStrikeCmd {
                actor: pc_id,
                target,
                command: Command::SwordstrikeThrustB,
                composite: None,
                gesture_quality: GestureQuality::PERFECT,
                with_seek: true,
                seek_distance: None,
            },
        );
        let seek = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .and_then(|sequence| sequence.get(0))
            .expect("legacy sword command launches its reconstructed seek");
        let SequenceElementData::Movement { tolerance, .. } = &seek.data else {
            panic!("legacy sword command did not launch movement")
        };
        assert_eq!(*tolerance, 72.0);
    }

    #[test]
    fn composite_sword_command_launches_two_quantized_strikes() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let target = spawn_pc_at(&mut engine, 90.0, 10.0);
        engine.apply_command(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::SwordStrikeCmd {
                actor: pc_id,
                target,
                command: Command::SwordstrikeThrustD,
                composite: Some(CompositeSwordTechnique::RisingFeint),
                gesture_quality: GestureQuality::GOOD,
                with_seek: false,
                seek_distance: None,
            },
        );

        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .find(|sequence| sequence.elements.len() == 2)
            .expect("composite strike sequence");
        assert_eq!(sequence.elements[0].command, Command::SwordstrikeThrustD);
        assert_eq!(sequence.elements[1].command, Command::SwordstrikeThrustB);
        assert_eq!(sequence.elements[0].command_level, 1);
        assert_eq!(sequence.elements[1].command_level, 2);
        assert!(
            sequence
                .elements
                .iter()
                .all(|element| element.gesture_quality == GestureQuality::GOOD)
        );
    }

    #[test]
    fn mismatched_composite_command_is_rejected() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let target = spawn_pc_at(&mut engine, 90.0, 10.0);
        engine.apply_command(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::SwordStrikeCmd {
                actor: pc_id,
                target,
                command: Command::SwordstrikeThrustA,
                composite: Some(CompositeSwordTechnique::RisingFeint),
                gesture_quality: GestureQuality::PERFECT,
                with_seek: false,
                seek_distance: None,
            },
        );
        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
    }

    #[test]
    fn disabled_composite_cannot_enter_an_automatic_quick_action() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Hit, 1)]);
        let target = spawn_pc_at(&mut engine, 90.0, 10.0);
        let sim = crate::sim_rng::test_context();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::SetCombatGestureRules {
                more_combat_gestures: false,
                gesture_quality_damage: true,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::QueueQuickAction {
                action: Action::Hit,
                command: PlayerCommand::SwordStrikeCmd {
                    actor: pc_id,
                    target,
                    command: CompositeSwordTechnique::RisingFeint.first_command(),
                    composite: Some(CompositeSwordTechnique::RisingFeint),
                    gesture_quality: GestureQuality::PERFECT,
                    with_seek: false,
                    seek_distance: None,
                }
                .into(),
            },
        );

        assert!(engine.players.auto_queues.is_empty(pc_id));
        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
    }

    #[test]
    fn disabled_quality_damage_rejects_reduced_strike_before_launch() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let target = spawn_pc_at(&mut engine, 90.0, 10.0);
        let sim = crate::sim_rng::test_context();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::SetCombatGestureRules {
                more_combat_gestures: true,
                gesture_quality_damage: false,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::SwordStrikeCmd {
                actor: pc_id,
                target,
                command: Command::SwordstrikeThrustA,
                composite: None,
                gesture_quality: GestureQuality::GOOD,
                with_seek: false,
                seek_distance: None,
            },
        );

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
    }

    #[test]
    fn explicitly_queued_sword_strike_still_records_a_quick_action() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Hit, 1)]);
        let target = spawn_pc_at(&mut engine, 90.0, 10.0);
        let busy = SequenceElement::new(1, Command::EnterListen, Some(pc_id));
        let busy_sequence = engine.orders.sequence_manager.launch_element(busy);
        engine
            .orders
            .sequence_manager
            .element_in_progress(busy_sequence, 0);

        engine.apply_command(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::QueueQuickAction {
                action: Action::Hit,
                command: PlayerCommand::SwordStrikeCmd {
                    actor: pc_id,
                    target,
                    command: Command::SwordstrikeThrustA,
                    composite: None,
                    gesture_quality: GestureQuality::PERFECT,
                    with_seek: false,
                    seek_distance: Some(0.0),
                }
                .into(),
            },
        );

        let entry = engine
            .players
            .auto_queues
            .get(pc_id)
            .and_then(|queue| queue.first())
            .expect("queued sword-strike entry");
        assert!(matches!(
            entry.step.replay,
            QaReplayCommand::SwordStrike {
                target: recorded_target,
                command: Command::SwordstrikeThrustA,
                composite: None,
                gesture_quality: GestureQuality::PERFECT,
                with_seek: false,
                seek_distance: Some(0.0),
            } if recorded_target == target
        ));
        assert!(entry.titbit.is_some());
        assert!(engine.players.macro_store.get(pc_id).is_none());
    }

    #[test]
    fn auto_launch_preserves_empty_manual_recording() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Whistle, 1)]);
        engine.players.seats[0].selection.push(pc_id);
        let mut busy = SequenceElement::new(1, Command::EnterListen, Some(pc_id));
        busy.priority = crate::sequence::SequencePriority::Normal;
        let busy_sequence = engine.orders.sequence_manager.launch_element(busy);
        engine
            .orders
            .sequence_manager
            .element_in_progress(busy_sequence, 0);
        let sim = crate::sim_rng::test_context();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::QueueQuickAction {
                action: Action::Whistle,
                command: PlayerCommand::LaunchSelfAbility {
                    actor: pc_id,
                    command: Command::WhistleCmd,
                }
                .into(),
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 1,
            },
        );
        let armed_state = engine
            .players
            .macro_store
            .get(pc_id)
            .expect("empty armed manual slot")
            .clone();

        engine
            .orders
            .sequence_manager
            .element_terminated(busy_sequence, 0);
        let mut camera = CameraDisplayState::default();
        engine.advance_auto_quick_action_queues(&sim, &mut camera, &assets);

        assert_eq!(
            engine.players.macro_store.get(pc_id),
            Some(&armed_state),
            "automatic execution must not capture into an empty armed manual slot"
        );
        assert!(engine.is_qa_recording_for(pc_id));
        assert!(engine.players.auto_queues.is_empty(pc_id));
        assert!(
            engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(pc_id, |command| {
                    command == Command::WhistleCmd
                }),
            "the automatic command must still launch"
        );
    }

    #[test]
    fn restored_auto_launch_preserves_occupied_manual_recording_and_titbit() {
        std::thread::Builder::new()
            .name("restored-auto-launch-snapshot".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(restored_auto_launch_preserves_occupied_manual_recording_and_titbit_inner)
            .expect("spawn large-stack snapshot regression")
            .join()
            .expect("large-stack snapshot regression panicked");
    }

    fn restored_auto_launch_preserves_occupied_manual_recording_and_titbit_inner() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Whistle, 1)]);
        engine.players.seats[0].selection.push(pc_id);
        let sim = crate::sim_rng::test_context();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchSelfAbility {
                actor: pc_id,
                command: Command::EnterListen,
            },
        );
        let manual_titbit = engine
            .players
            .macro_store
            .get(pc_id)
            .and_then(|state| state.get_slot_titbit(0))
            .expect("occupied manual slot titbit");
        let manual_icon = engine
            .get_entity(pc_id)
            .and_then(Entity::pc_data)
            .expect("test PC")
            .portrait
            .quick_icons[0];

        let mut busy = SequenceElement::new(1, Command::EnterListen, Some(pc_id));
        busy.priority = crate::sequence::SequencePriority::Normal;
        let busy_sequence = engine.orders.sequence_manager.launch_element(busy);
        engine
            .orders
            .sequence_manager
            .element_in_progress(busy_sequence, 0);
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::QueueQuickAction {
                action: Action::Whistle,
                command: PlayerCommand::LaunchSelfAbility {
                    actor: pc_id,
                    command: Command::WhistleCmd,
                }
                .into(),
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        let armed_state = engine
            .players
            .macro_store
            .get(pc_id)
            .expect("occupied armed manual slot")
            .clone();

        let bytes = crate::engine::snapshot::encode_native_engine_inner(&engine);
        let mut engine = crate::engine::snapshot::decode_native_engine_inner(&bytes)
            .expect("restore engine with automatic and manual QA state");
        assert_eq!(engine.players.macro_store.get(pc_id), Some(&armed_state));

        engine
            .orders
            .sequence_manager
            .element_terminated(busy_sequence, 0);
        let mut restored_camera = CameraDisplayState::default();
        engine.advance_auto_quick_action_queues(&sim, &mut restored_camera, &assets);

        assert_eq!(engine.players.macro_store.get(pc_id), Some(&armed_state));
        assert!(engine.is_qa_recording_for(pc_id));
        assert_eq!(
            engine
                .players
                .macro_store
                .get(pc_id)
                .and_then(|state| state.get_slot_titbit(0)),
            Some(manual_titbit)
        );
        assert!(
            engine
                .feedback
                .titbit_manager
                .titbits()
                .iter()
                .any(|titbit| titbit.id == manual_titbit)
        );
        let restored_icon = engine
            .get_entity(pc_id)
            .and_then(Entity::pc_data)
            .expect("restored test PC")
            .portrait
            .quick_icons[0];
        assert_eq!(restored_icon.titbit_id, manual_icon.titbit_id);
        assert_eq!(restored_icon.running, manual_icon.running);
        assert!(engine.players.auto_queues.is_empty(pc_id));
        assert!(
            engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(pc_id, |command| {
                    command == Command::WhistleCmd
                }),
            "the restored automatic command must still launch"
        );
    }

    #[test]
    fn shift_queue_starts_first_action_and_keeps_later_action_visible() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Whistle, 1)]);
        engine.players.seats[0].selection.push(pc_id);
        let manual_step = QuickActionStep {
            action: Action::Bow,
            position: MapPoint::new(123.0, 456.0),
            replay: QaReplayCommand::Move {
                destination: MapPoint::new(123.0, 456.0),
                running: false,
                route: crate::macro_store::RecordedQaMoveRoute {
                    goal_sector: crate::sector::SectorNumber::new(1),
                    goal_sector_index: crate::fast_find_grid::SectorIndex::new(0)
                        .expect("valid test sector index"),
                    goal_layer: 0,
                },
            },
        };
        let manual = engine.players.macro_store.get_or_insert(pc_id);
        manual.begin_recording(0);
        manual.append_if_recording(manual_step.clone());
        manual.stop_recording();
        let queued = PlayerCommand::QueueQuickAction {
            action: Action::Whistle,
            command: PlayerCommand::LaunchSelfAbility {
                actor: pc_id,
                command: Command::WhistleCmd,
            }
            .into(),
        };
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        let sim = crate::sim_rng::test_context();

        // PCs retain a wait-priority idle card in real missions. It must not
        // postpone the first automatic QA until some unrelated live command
        // happens to interrupt that card.
        let mut idle = SequenceElement::new(1, Command::Wait, Some(pc_id));
        idle.priority = crate::sequence::SequencePriority::Wait;
        let idle_sequence = engine.orders.sequence_manager.launch_element(idle);
        engine
            .orders
            .sequence_manager
            .element_in_progress(idle_sequence, 0);

        // Door/lift traversal can leave an interrupted command postponed after
        // the movement itself has settled. A postponed card is dormant, not
        // executable actor work, and must not pin the automatic queue forever.
        let stale = SequenceElement::new(1, Command::EnterListen, Some(pc_id));
        let stale_sequence = engine.orders.sequence_manager.launch_element(stale);
        engine
            .orders
            .sequence_manager
            .postpone_element(stale_sequence, 0);

        engine.apply_command(&sim, &mut display, &mut input, &assets, &queued);
        assert!(engine.players.auto_queue_active.contains(&pc_id));
        assert!(engine.has_quick_action(pc_id, 0));
        assert!(engine.players.auto_queues.is_empty(pc_id));
        assert_eq!(
            engine
                .players
                .macro_store
                .get(pc_id)
                .and_then(|state| state.slot(0))
                .expect("manual slot")
                .steps,
            vec![manual_step.clone()]
        );
        assert!(
            engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(pc_id, |command| {
                    command == Command::WhistleCmd
                })
        );

        engine.apply_command(&sim, &mut display, &mut input, &assets, &queued);
        assert!(engine.has_quick_action(pc_id, 0));
        let queue = engine
            .players
            .auto_queues
            .get(pc_id)
            .expect("queued QA state");
        assert_eq!(queue.len(), 1);
        assert!(queue[0].titbit.is_some());
        assert_eq!(
            engine
                .get_entity(pc_id)
                .and_then(Entity::pc_data)
                .expect("test PC")
                .current_action,
            Action::NoAction,
            "queueing must not arm the live PC action"
        );

        let (sequence_id, element_index) = engine
            .orders
            .sequence_manager
            .live_element_for_actor_matching(pc_id, |element| {
                element.command == Command::WhistleCmd
            })
            .expect("first queued action is live");
        engine
            .orders
            .sequence_manager
            .element_terminated(sequence_id, element_index);
        let mut camera = CameraDisplayState::default();
        engine.advance_auto_quick_action_queues(&sim, &mut camera, &assets);

        assert!(engine.has_quick_action(pc_id, 0));
        assert_eq!(
            engine
                .players
                .macro_store
                .get(pc_id)
                .and_then(|state| state.slot(0))
                .expect("manual slot after automatic replay")
                .steps,
            vec![manual_step]
        );
        assert!(engine.players.auto_queue_active.contains(&pc_id));
        assert!(
            engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(pc_id, |command| {
                    command == Command::WhistleCmd
                }),
            "the pending QA starts as soon as the preceding action terminates"
        );
    }

    #[test]
    fn shift_queue_retains_more_than_three_pending_actions() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Whistle, 1)]);
        let mut busy = SequenceElement::new(1, Command::EnterListen, Some(pc_id));
        busy.priority = crate::sequence::SequencePriority::Normal;
        let busy_sequence = engine.orders.sequence_manager.launch_element(busy);
        engine
            .orders
            .sequence_manager
            .element_in_progress(busy_sequence, 0);

        let queued = PlayerCommand::QueueQuickAction {
            action: Action::Whistle,
            command: PlayerCommand::LaunchSelfAbility {
                actor: pc_id,
                command: Command::WhistleCmd,
            }
            .into(),
        };
        let sim = crate::sim_rng::test_context();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        for _ in 0..6 {
            engine.apply_command(&sim, &mut display, &mut input, &assets, &queued);
        }

        assert!(engine.players.macro_store.get(pc_id).is_none());
        assert_eq!(engine.players.auto_queues.len(pc_id), 6);
        let queue = engine.players.auto_queues.get(pc_id).expect("auto queue");
        assert!(queue.iter().all(|entry| entry.titbit.is_some()));
    }

    #[test]
    fn quick_action_tail_rewrites_only_unequipped_final_bow_shot() {
        assert_eq!(
            quick_action_tail_command(Command::ShootBow, true, false),
            Command::ShootBowOnce
        );
        assert_eq!(
            quick_action_tail_command(Command::ShootBow, true, true),
            Command::ShootBow
        );
        assert_eq!(
            quick_action_tail_command(Command::ShootBow, false, false),
            Command::ShootBow
        );
        assert_eq!(
            quick_action_tail_command(Command::Take, true, false),
            Command::Take
        );
    }

    #[test]
    fn queued_bow_shot_starts_once_after_real_work_ends_despite_postponed_card() {
        let (mut engine, mut assets, pc_id) = setup_pc_engine(&[(Action::Bow, 1)]);
        let target = spawn_pc_at(&mut engine, 90.0, 10.0);
        configure_valid_bow_quick_action(&mut engine, &mut assets, pc_id, target);
        let busy = SequenceElement::new(1, Command::EnterListen, Some(pc_id));
        let busy_sequence = engine.orders.sequence_manager.launch_element(busy);
        engine
            .orders
            .sequence_manager
            .element_in_progress(busy_sequence, 0);

        let stale = SequenceElement::new(1, Command::LeaveListen, Some(pc_id));
        let stale_sequence = engine.orders.sequence_manager.launch_element(stale);
        engine
            .orders
            .sequence_manager
            .postpone_element(stale_sequence, 0);

        let sim = crate::sim_rng::test_context();
        let mut display = HostDisplayState::default();
        engine.apply_command(
            &sim,
            &mut display,
            &mut InputState::default(),
            &assets,
            &PlayerCommand::QueueQuickAction {
                action: Action::Bow,
                command: PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target,
                    command: Command::ShootBow,
                    running: false,
                }
                .into(),
            },
        );
        assert_eq!(engine.players.auto_queues.len(pc_id), 1);
        assert!(!engine.has_quick_action(pc_id, 0));

        engine
            .orders
            .sequence_manager
            .element_terminated(busy_sequence, 0);
        let mut camera = CameraDisplayState::default();
        engine.advance_auto_quick_action_queues(&sim, &mut camera, &assets);

        assert!(engine.players.auto_queues.is_empty(pc_id));
        assert!(!engine.has_quick_action(pc_id, 0));
        assert!(
            engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(pc_id, |command| {
                    command == Command::ShootBowOnce
                }),
            "a queued bow interaction from an unequipped posture must launch as one shot"
        );
        assert!(
            !engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(pc_id, |command| {
                    command == Command::ShootBow
                }),
            "ordinary ShootBow reloads and leaves the bow equipped"
        );
    }

    #[test]
    fn queued_pickup_moves_following_bow_preview_origin_to_pickup_target() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Bow, 2)]);
        let pickup = spawn_bonus(&mut engine, ObjectType::BonusArrow, true, Action::Bow);
        let pickup_position = MapPoint::new(420.0, 730.0);
        engine
            .get_entity_mut(pickup)
            .expect("pickup")
            .element_data_mut()
            .set_position_map(pickup_position);
        let bow_target = spawn_pc_at(&mut engine, 900.0, 730.0);

        let busy = SequenceElement::new(1, Command::EnterListen, Some(pc_id));
        let busy_sequence = engine.orders.sequence_manager.launch_element(busy);
        engine
            .orders
            .sequence_manager
            .element_in_progress(busy_sequence, 0);

        let sim = crate::sim_rng::test_context();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        for (action, command) in [
            (
                Action::NoAction,
                PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target: pickup,
                    command: Command::Take,
                    running: false,
                },
            ),
            (
                Action::Bow,
                PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target: bow_target,
                    command: Command::ShootBow,
                    running: false,
                },
            ),
        ] {
            engine.apply_command(
                &sim,
                &mut display,
                &mut input,
                &assets,
                &PlayerCommand::QueueQuickAction {
                    action,
                    command: command.into(),
                },
            );
        }

        assert_eq!(engine.planned_action_origin(pc_id), Some(pickup_position));
    }

    #[test]
    fn shift_pickup_uses_take_quick_action_phase() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Bow, 4)]);
        let target = spawn_bonus(&mut engine, ObjectType::BonusArrow, true, Action::Bow);
        let busy = SequenceElement::new(1, Command::EnterListen, Some(pc_id));
        let busy_sequence = engine.orders.sequence_manager.launch_element(busy);
        engine
            .orders
            .sequence_manager
            .element_in_progress(busy_sequence, 0);

        engine.apply_command(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::QueueQuickAction {
                action: Action::NoAction,
                command: PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target,
                    command: Command::Take,
                    running: false,
                }
                .into(),
            },
        );

        let titbit = engine
            .players
            .auto_queues
            .get(pc_id)
            .and_then(|queue| queue.first())
            .and_then(|entry| entry.titbit)
            .expect("pickup QA titbit");
        assert_eq!(
            engine.feedback.titbit_manager.get_phase(titbit),
            Some(crate::titbit::QuickAction::Take as u16)
        );
    }

    #[test]
    fn resolved_throw_orientation_targets_only_the_recorded_pc() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Stone, 1)]);
        let actor_position = WorldPoint3D::new(242.0, 2329.0, 90.0);
        engine
            .get_entity_mut(pc_id)
            .unwrap()
            .element_data_mut()
            .set_position(actor_position);
        {
            let sprite = &mut engine
                .get_entity_mut(pc_id)
                .unwrap()
                .element_data_mut()
                .sprite;
            sprite.force_sprite_row_raw(78);
            sprite.current_frame = 9;
        }
        let target = WorldPoint3D::new(435.0, 2329.0, 274.0);

        engine.perform_resolved_orientation(&assets, pc_id, Action::Stone, MapPoint::ZERO, target);

        let element = engine.get_entity(pc_id).unwrap().element_data();
        assert_eq!(
            i16::from(element.sprite.position_iface.get_direction_goal().as_u8()),
            crate::position_interface::vector_to_sector_0_to_15_iso(
                target.x - actor_position.x,
                target.y - actor_position.y,
            )
        );
        assert_eq!(element.sprite.current_row, 78);
        assert_eq!(element.sprite.current_frame, 9);
    }

    #[test]
    fn late_popup_purse_orientation_preserves_the_pre_turn_sprite_row() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Purse, 3)]);
        let entity = engine.get_entity_mut(pc_id).expect("popup purse PC");
        entity
            .position_iface_mut()
            .set_direction_instantly(crate::position_interface::Direction::from_raw(1));
        entity.element_data_mut().sprite.force_sprite_row_raw(1);

        // Action processing has already selected row 1. The popup's
        // nested Refresh then turns once toward east without selecting a new
        // animation row.
        engine.perform_resolved_orientation(
            &assets,
            pc_id,
            Action::Purse,
            MapPoint::ZERO,
            WorldPoint3D::new(100.0, 0.0, 0.0),
        );

        let entity = engine.get_entity(pc_id).expect("popup purse PC survives");
        assert_eq!(u8::from(entity.position_iface().get_direction()), 2);
        assert_eq!(entity.sprite().current_row, 1);
    }

    #[test]
    fn late_popup_bow_orientation_preserves_the_old_direction_row() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Bow, 4)]);
        let entity = engine.get_entity_mut(pc_id).expect("popup bow PC");
        entity
            .position_iface_mut()
            .set_direction_instantly(crate::position_interface::Direction::from_raw(1));
        // AimingWithBow's conversion base in the captured profile is 1664;
        // Action processing selected base + old direction before nested refresh.
        entity.element_data_mut().sprite.force_sprite_row_raw(1665);

        engine.perform_resolved_orientation(
            &assets,
            pc_id,
            Action::Bow,
            MapPoint::ZERO,
            WorldPoint3D::new(100.0, 0.0, 0.0),
        );

        let entity = engine.get_entity(pc_id).expect("popup bow PC survives");
        assert_eq!(u8::from(entity.position_iface().get_direction()), 2);
        assert_eq!(entity.sprite().current_row, 1665);
    }

    fn setup_pc_engine_with_split_profile_and_status(
        actions: &[(Action, u16)],
    ) -> (EngineInner, LevelAssets, EntityId) {
        let mut actions_arr = [Action::NoAction; crate::profiles::NUMBER_OF_PC_ACTIONS];
        let mut max_ammo_arr = [0u16; crate::profiles::NUMBER_OF_PC_ACTIONS];
        for (i, (a, m)) in actions.iter().enumerate() {
            actions_arr[i] = *a;
            max_ammo_arr[i] = *m;
        }

        let profile_idx = crate::profiles::CharacterProfileIdx(2);
        let description_idx = 1u32;

        let mut pm = ProfileManager::new();
        pm.characters.push(CharacterProfile::default());
        pm.characters.push(CharacterProfile::default());
        pm.characters.push(CharacterProfile {
            actions: actions_arr,
            action_max_ammo: max_ammo_arr,
            ..CharacterProfile::default()
        });
        let mut assets = LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(pm);

        let mut engine = EngineInner::new();
        let mut campaign = crate::campaign::Campaign::default();
        let mut other_description = crate::campaign::PcDescription {
            character_profile_idx: Some(profile_idx),
            instanced: false,
            ..Default::default()
        };
        other_description.status.num_arrows = 12;
        campaign.characters.push(other_description);
        campaign.characters.push(crate::campaign::PcDescription {
            character_profile_idx: Some(profile_idx),
            instanced: true,
            ..Default::default()
        });
        engine.mission_domain.campaign = campaign;

        let pc_id = engine.add_entity(Entity::Pc(ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                profile_index: profile_idx,
                // Deliberately independent identities: neither the first
                // matching profile nor the list index owns this actor's status.
                list_index: 0,
                campaign_description_index: Some(description_idx),
                life_points: 50,
                ..PcData::default()
            },
        }));

        (engine, assets, pc_id)
    }

    fn spawn_bonus(
        engine: &mut EngineInner,
        object_type: ObjectType,
        active: bool,
        assoc: Action,
    ) -> EntityId {
        engine.add_entity(Entity::Bonus(ElementBonus {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ObjectBonus;
                initial_element.active = active;
                initial_element
            },
            object: ObjectData {
                object_type,
                associated_action: assoc,
                ..Default::default()
            },
        }))
    }

    fn spawn_scroll(engine: &mut EngineInner, active: bool) -> EntityId {
        engine.add_entity(Entity::Scroll(ElementScroll {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ObjectScroll;
                initial_element.active = active;
                initial_element
            },
            object: ObjectData {
                object_type: ObjectType::Scroll,
                ..Default::default()
            },
            ..Default::default()
        }))
    }

    fn spawn_projectile(
        engine: &mut EngineInner,
        object_type: ObjectType,
        flying: bool,
        assoc: Action,
    ) -> EntityId {
        engine.add_entity(Entity::Projectile(ElementProjectile {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ObjectProjectile;
                initial_element.active = true;
                initial_element
            },
            object: ObjectData {
                object_type,
                associated_action: assoc,
                ..Default::default()
            },
            projectile: ProjectileData {
                flying,
                ..Default::default()
            },
        }))
    }

    fn spawn_net(engine: &mut EngineInner, flying: bool) -> EntityId {
        let mut element = {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ObjectNet;
            initial_element.active = true;
            initial_element
        };
        element.set_position(WorldPoint3D::default());
        engine.add_entity(Entity::Net(ElementNet {
            element,
            object: ObjectData {
                associated_action: Action::Net,
                object_type: ObjectType::Net,
                ..Default::default()
            },
            projectile: ProjectileData {
                flying,
                ..Default::default()
            },
            net: NetData::default(),
        }))
    }

    fn bind_single_action_point(
        engine: &mut EngineInner,
        id: EntityId,
        action: crate::order::OrderType,
        hotspot: crate::coordinates::SpriteLocalPoint,
        center: crate::coordinates::SpriteAnchor,
    ) {
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = vec![UNMAPPED; crate::sprite_script::NONANIMATION_END];
        conversion[action as usize] = 0;
        let mut sprite = Sprite::new(
            std::sync::Arc::new(vec![script]),
            std::sync::Arc::new(conversion),
        );
        sprite.center = center;
        let element = engine.get_entity_mut(id).unwrap().element_data_mut();
        let position = element.position_map();
        let direction = element.direction();
        let pathfinder_index = element.sprite.position_iface.get_pathfinder_index();
        element.sprite = sprite;
        element.set_position_map(position);
        element.set_direction_instantly(direction);
        if let Some(pathfinder_index) = pathfinder_index {
            element
                .sprite
                .position_iface
                .set_pathfinder_index(pathfinder_index);
        }
    }

    fn setup_take_corpse_macro_scene(
        target_x: f32,
    ) -> (EngineInner, LevelAssets, EntityId, EntityId) {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let sector = crate::position_interface::SectorHandle::new(1);
        {
            let pc = engine.get_entity_mut(pc_id).expect("test PC exists");
            pc.element_data_mut()
                .publish_order_posture(Posture::HelpingToClimb);
            pc.element_data_mut()
                .set_position_map(crate::coordinates::MapPoint::new(100.0, 100.0));
            pc.element_data_mut().set_sector(sector);
        }
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::TransitionWaitingUprightCarryingCorpse,
            crate::coordinates::SpriteLocalPoint::new(25.0, 0.0),
            crate::coordinates::SpriteAnchor::new(0.0, 0.0),
        );
        engine
            .get_entity_mut(pc_id)
            .expect("test PC exists after sprite binding")
            .element_data_mut()
            .set_sector(sector);

        let mut corpse = ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Lying);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        corpse
            .element
            .set_position_map(crate::coordinates::MapPoint::new(target_x, 100.0));
        corpse.element.set_sector(sector);
        let corpse_id = engine.add_entity(Entity::Pc(corpse));

        let state = engine.players.macro_store.get_or_insert(pc_id);
        state.begin_recording(0);
        state.append_if_recording(QuickActionStep {
            action: Action::NoAction,
            position: crate::coordinates::MapPoint::new(target_x, 100.0),
            replay: QaReplayCommand::Interaction {
                target: corpse_id,
                command: Command::TakeCorpse,
                double_click: false,
            },
        });
        state.stop_recording();

        (engine, assets, pc_id, corpse_id)
    }

    fn start_macro(engine: &mut EngineInner, assets: &LevelAssets, pc_id: EntityId) {
        let sim = crate::sim_rng::test_context();
        engine.apply_command(
            &sim,
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            assets,
            &PlayerCommand::StartMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
    }

    #[test]
    fn take_corpse_macro_embeds_helping_recovery_after_near_interaction() {
        let (mut engine, assets, pc_id, _corpse_id) = setup_take_corpse_macro_scene(110.0);

        start_macro(&mut engine, &assets, pc_id);

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("macro launches one interaction route");
        let commands: Vec<_> = sequence
            .elements
            .iter()
            .map(|element| element.command)
            .collect();
        assert_eq!(
            commands,
            [Command::TakeCorpse, Command::EnterHelpingClimb],
            "quick-action startup appends posture recovery to the recorded sequence"
        );
        assert_eq!(sequence.elements[0].command_level, 1);
        assert_eq!(sequence.elements[1].command_level, 2);
    }

    #[test]
    fn take_corpse_macro_embeds_helping_recovery_in_far_post_seek() {
        let (mut engine, assets, pc_id, _corpse_id) = setup_take_corpse_macro_scene(180.0);

        start_macro(&mut engine, &assets, pc_id);

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("macro launches one seek route");
        let seek = sequence.get(0).expect("route begins with Seek");
        assert_eq!(seek.command, Command::Seek);
        let SequenceElementData::Movement {
            post_seek_sequence, ..
        } = &seek.data
        else {
            panic!("TakeCorpse macro route must begin with movement");
        };
        let post_seek = post_seek_sequence
            .as_ref()
            .expect("TakeCorpse remains attached to Seek");
        let commands: Vec<_> = post_seek
            .elements
            .iter()
            .map(|element| element.command)
            .collect();
        assert_eq!(commands, [Command::TakeCorpse, Command::EnterHelpingClimb]);
    }

    #[test]
    fn ordinary_take_corpse_does_not_add_macro_posture_recovery() {
        let (mut engine, assets, pc_id, corpse_id) = setup_take_corpse_macro_scene(110.0);
        engine
            .players
            .macro_store
            .get_or_insert(pc_id)
            .clear_slot(0);
        let sim = crate::sim_rng::test_context();

        engine.apply_command(
            &sim,
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target: corpse_id,
                command: Command::TakeCorpse,
                running: false,
            },
        );

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("ordinary interaction launches one route");
        assert_eq!(sequence.len(), 1);
        assert_eq!(sequence.get(0).unwrap().command, Command::TakeCorpse);
    }

    fn setup_drop_ale_macro_scene() -> (EngineInner, LevelAssets, EntityId) {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Ale, 1)]);
        {
            let pc = engine.get_entity_mut(pc_id).expect("test PC exists");
            pc.element_data_mut()
                .publish_order_posture(Posture::HelpingToClimb);
            pc.element_data_mut()
                .set_position_map(crate::coordinates::MapPoint::new(20.0, 30.0));
        }
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::DroppingAle,
            crate::coordinates::SpriteLocalPoint::new(13.0, 0.0),
            crate::coordinates::SpriteAnchor::new(0.0, 0.0),
        );

        let target_pos = crate::coordinates::MapPoint::new(80.0, 90.0);
        let state = engine.players.macro_store.get_or_insert(pc_id);
        state.begin_recording(0);
        state.append_if_recording(QuickActionStep {
            action: Action::Ale,
            position: target_pos,
            replay: QaReplayCommand::DropAle {
                target_pos,
                running: false,
                already_authorized: false,
                goal_override: None,
                goal_sector_index_override: None,
                recorded_gate_path: None,
            },
        });
        state.stop_recording();

        (engine, assets, pc_id)
    }

    fn drop_ale_post_seek_commands(engine: &EngineInner) -> Vec<Command> {
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("DropAle launches one seek route");
        let seek = sequence.get(0).expect("DropAle route begins with Seek");
        assert_eq!(seek.command, Command::Seek);
        let SequenceElementData::Movement {
            post_seek_sequence, ..
        } = &seek.data
        else {
            panic!("DropAle route must begin with movement");
        };
        post_seek_sequence
            .as_ref()
            .expect("DropAle remains attached to Seek")
            .elements
            .iter()
            .map(|element| element.command)
            .collect()
    }

    fn drop_ale_seek_goal(
        engine: &EngineInner,
    ) -> (
        crate::coordinates::MapPoint,
        Option<crate::position_interface::SectorHandle>,
        u16,
    ) {
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("DropAle launches one seek route");
        let seek = sequence.get(0).expect("DropAle route begins with Seek");
        let SequenceElementData::Movement {
            destination,
            sector,
            layer,
            ..
        } = &seek.data
        else {
            panic!("DropAle route must begin with movement");
        };
        (*destination, *sector, *layer)
    }

    fn setup_drop_ale_sector_identity_scene() -> (
        EngineInner,
        LevelAssets,
        EntityId,
        crate::fast_find_grid::SectorIndex,
        crate::fast_find_grid::SectorIndex,
    ) {
        use crate::fast_find_grid::{GridSector, SectorIndex};
        use crate::sector::{SectorNumber, SectorType};

        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Ale, 1)]);
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::DroppingAle,
            crate::coordinates::SpriteLocalPoint::new(13.0, 0.0),
            crate::coordinates::SpriteAnchor::new(0.0, 0.0),
        );

        let sector = |min_x, max_x| GridSector {
            points: vec![
                crate::coordinates::MapPoint::new(min_x, 0.0),
                crate::coordinates::MapPoint::new(max_x, 0.0),
                crate::coordinates::MapPoint::new(max_x, 128.0),
                crate::coordinates::MapPoint::new(min_x, 128.0),
            ],
            bounding_box: crate::coordinates::MapBBox::from_coords(min_x, 0.0, max_x, 128.0),
            sector_type: SectorType::MOTION | SectorType::AREA | SectorType::MOUSE,
            layer: 0,
            // Pc130's failure used two live arena objects whose public
            // sector number was the same. Exact sector identity matters here.
            sector_number: SectorNumber::new(0),
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
        };

        engine.world.fast_grid_mut().size_map(4, 2);
        engine.world.fast_grid_mut().allocate_layers(1);
        let source = SectorIndex::new(
            engine
                .world
                .fast_grid_mut()
                .add_sector(sector(0.0, 127.0), 0),
        )
        .expect("source sector index");
        let alias = SectorIndex::new(
            engine
                .world
                .fast_grid_mut()
                .add_sector(sector(128.0, 255.0), 0),
        )
        .expect("alias sector index");

        let pc = engine.get_entity_mut(pc_id).expect("test PC exists");
        pc.element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(20.0, 30.0));
        pc.position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -6.0, -4.0, 6.0, 4.0,
            ));
        pc.element_data_mut().set_sector(Some(
            crate::position_interface::SectorHandle::new(0)
                .unwrap()
                .with_arena_index(source),
        ));

        (engine, assets, pc_id, source, alias)
    }

    #[test]
    fn drop_ale_same_sector_retains_exact_identity_and_installs_move_ok() {
        let (mut engine, assets, pc_id, source, _) = setup_drop_ale_sector_identity_scene();
        let destination = crate::coordinates::MapPoint::new(80.0, 90.0);

        engine.apply_drop_ale_at(pc_id, destination, false, false, None, None, None);
        let (_, goal, layer) = drop_ale_seek_goal(&engine);
        assert_eq!(goal.and_then(|sector| sector.arena_index()), Some(source));
        assert_eq!(layer, 0);

        engine.hourglass_phase_sequences(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &assets,
        );

        let actor = engine
            .get_entity(pc_id)
            .and_then(|entity| entity.actor_data())
            .expect("DropAle owner remains an actor");
        assert_eq!(
            actor.installed_order.map(|order| order.order_type),
            Some(crate::order::OrderType::TransitionWaitingUprightWalkingUpright)
        );
        let (sequence_id, element_index) = engine
            .orders
            .sequence_manager
            .current_element_for_actor(pc_id)
            .expect("same-sector DropAle installs its direct movement");
        let movement = engine
            .orders
            .sequence_manager
            .get_element(sequence_id, element_index)
            .expect("selected DropAle movement exists");
        assert_eq!(movement.command, Command::MoveOk);
        assert_eq!(
            movement.current_order().map(|order| order.order_type),
            Some(crate::order::OrderType::TransitionWaitingUprightWalkingUpright)
        );
        assert!(
            movement
                .orders
                .iter()
                .any(|order| order.order_type == crate::order::OrderType::WalkingUpright),
            "direct same-sector movement must retain its eventual walking order"
        );
    }

    #[test]
    fn drop_ale_duplicate_public_sector_keeps_cross_sector_identity() {
        let (mut engine, _, pc_id, source, alias) = setup_drop_ale_sector_identity_scene();
        let destination = crate::coordinates::MapPoint::new(180.0, 90.0);

        engine.apply_drop_ale_at(pc_id, destination, false, false, None, None, None);

        let (_, goal, layer) = drop_ale_seek_goal(&engine);
        let goal = goal.expect("DropAle target must resolve to a sector");
        assert_eq!(u16::from(goal), 0);
        assert_eq!(goal.arena_index(), Some(alias));
        assert_ne!(goal.arena_index(), Some(source));
        assert_eq!(layer, 0);
    }

    #[test]
    fn drop_ale_patch_goal_retains_exact_underlying_sector_identity() {
        use crate::fast_find_grid::GridSector;
        use crate::sector::{SectorNumber, SectorType};

        let (mut engine, _, pc_id, source, _) = setup_drop_ale_sector_identity_scene();
        let destination = crate::coordinates::MapPoint::new(80.0, 90.0);
        engine.world.fast_grid_mut().add_sector(
            GridSector {
                points: vec![
                    crate::coordinates::MapPoint::new(64.0, 64.0),
                    crate::coordinates::MapPoint::new(96.0, 64.0),
                    crate::coordinates::MapPoint::new(96.0, 112.0),
                    crate::coordinates::MapPoint::new(64.0, 112.0),
                ],
                bounding_box: crate::coordinates::MapBBox::from_coords(64.0, 64.0, 96.0, 112.0),
                sector_type: SectorType::PATCH | SectorType::AREA | SectorType::MOUSE,
                layer: 0,
                sector_number: SectorNumber::new(77),
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
                underlying_sector: Some(source),
            },
            0,
        );

        engine.apply_drop_ale_at(pc_id, destination, false, false, None, None, None);

        let (_, goal, layer) = drop_ale_seek_goal(&engine);
        let goal = goal.expect("DropAle patch target resolves through its underlying sector");
        assert_eq!(u16::from(goal), 0);
        assert_eq!(goal.arena_index(), Some(source));
        assert_eq!(layer, 0);
    }

    #[test]
    fn drop_ale_macro_embeds_helping_recovery_in_post_seek() {
        let (mut engine, assets, pc_id) = setup_drop_ale_macro_scene();

        start_macro(&mut engine, &assets, pc_id);

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);
        assert_eq!(
            drop_ale_post_seek_commands(&engine),
            [Command::DropAle, Command::EnterHelpingClimb]
        );
    }

    #[test]
    fn ordinary_drop_ale_does_not_add_macro_posture_recovery() {
        let (mut engine, assets, pc_id) = setup_drop_ale_macro_scene();
        engine
            .players
            .macro_store
            .get_or_insert(pc_id)
            .clear_slot(0);
        let target_pos = crate::coordinates::MapPoint::new(80.0, 90.0);

        engine.apply_command(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::DropAleAt {
                actor: pc_id,
                target_pos,
                running: false,
                already_authorized: false,
                goal_override: None,
                goal_sector_index_override: None,
                recorded_gate_path: None,
            },
        );

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);
        assert_eq!(drop_ale_post_seek_commands(&engine), [Command::DropAle]);
        assert_eq!(drop_ale_seek_goal(&engine).0, target_pos);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .next()
                .and_then(|sequence| sequence.elements.first())
                .map(|element| element.point_seek_route_provenance),
            Some(crate::sequence::PointSeekRouteProvenance::Live),
            "live DropAle keeps reconstructed gate search as its fallback"
        );
    }

    #[test]
    fn resolved_replay_drop_ale_preserves_authorized_point_and_route_goal() {
        let (mut engine, assets, pc_id, source_index, goal_index) =
            setup_drop_ale_sector_identity_scene();
        let authorized = crate::coordinates::MapPoint::new(2_607.467, 881.610_5);
        let recorded_gate_path = crate::gate::RecordedGatePath {
            source_sector: crate::sector::SectorNumber::new(0),
            source_sector_index: Some(source_index),
            source_layer: 0,
            outcome: crate::gate::RecordedGateOutcome::Success(vec![crate::gate::GatePathStep {
                door_index: crate::gate::DoorIndex::new(42).expect("valid door index"),
                direct: false,
            }]),
        };

        engine.apply_command(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::DropAleAt {
                actor: pc_id,
                target_pos: authorized,
                running: false,
                already_authorized: true,
                goal_override: Some((crate::sector::SectorNumber::new(0), 0)),
                goal_sector_index_override: Some(goal_index),
                recorded_gate_path: Some(recorded_gate_path.clone()),
            },
        );

        assert_eq!(
            drop_ale_seek_goal(&engine),
            (
                authorized,
                crate::position_interface::SectorHandle::new(0)
                    .map(|sector| sector.with_arena_index(goal_index)),
                0,
            )
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .next()
                .and_then(|sequence| sequence.elements.first())
                .and_then(|element| element.recorded_gate_path.as_ref()),
            Some(&recorded_gate_path),
            "the authoritative route must survive until cross-sector Seek expansion"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .next()
                .and_then(|sequence| sequence.elements.first())
                .map(|element| element.point_seek_route_provenance),
            Some(crate::sequence::PointSeekRouteProvenance::OriginalReplay),
        );
    }

    #[test]
    fn recording_live_drop_ale_resolves_same_and_cross_sector_goals_before_storage() {
        for (label, target, expected_goal) in [
            (
                "same-sector",
                crate::coordinates::MapPoint::new(80.0, 90.0),
                false,
            ),
            (
                "cross-sector duplicate-public-number",
                crate::coordinates::MapPoint::new(180.0, 90.0),
                true,
            ),
        ] {
            let sim = crate::sim_rng::test_context();
            let (mut engine, assets, pc_id, source_index, goal_index) =
                setup_drop_ale_sector_identity_scene();
            let mut display = HostDisplayState::default();
            let mut input = InputState::default();

            engine.apply_command(
                &sim,
                &mut display,
                &mut input,
                &assets,
                &PlayerCommand::StartRecordingMacro {
                    pc: Some(pc_id),
                    slot: 0,
                },
            );
            engine.apply_command(
                &sim,
                &mut display,
                &mut input,
                &assets,
                &PlayerCommand::DropAleAt {
                    actor: pc_id,
                    target_pos: target,
                    running: false,
                    already_authorized: false,
                    goal_override: None,
                    goal_sector_index_override: None,
                    recorded_gate_path: None,
                },
            );

            assert!(!engine.is_recording_macro(), "{label}");
            assert_eq!(
                engine.orders.sequence_manager.sequence_count(),
                0,
                "{label}"
            );
            let step = &engine
                .players
                .macro_store
                .get(pc_id)
                .and_then(|state| state.slot(0))
                .unwrap_or_else(|| panic!("{label} DropAle recording occupies slot zero"))
                .steps[0];
            let QaReplayCommand::DropAle {
                target_pos,
                already_authorized,
                goal_override,
                goal_sector_index_override,
                recorded_gate_path,
                ..
            } = &step.replay
            else {
                panic!("{label} recording stored a non-DropAle step")
            };
            assert_eq!(*target_pos, target, "{label}");
            assert!(*already_authorized, "{label}");
            assert_eq!(
                *goal_override,
                Some((crate::sector::SectorNumber::new(0), 0)),
                "{label}"
            );
            assert_eq!(
                *goal_sector_index_override,
                Some(if expected_goal {
                    goal_index
                } else {
                    source_index
                }),
                "{label}"
            );
            let recorded_goal_index = *goal_sector_index_override;
            assert_eq!(*recorded_gate_path, None, "{label}");

            engine.apply_command(
                &sim,
                &mut display,
                &mut input,
                &assets,
                &PlayerCommand::StartMacro {
                    pc: Some(pc_id),
                    slot: 0,
                },
            );
            let (_, replayed_goal, replayed_layer) = drop_ale_seek_goal(&engine);
            assert_eq!(
                replayed_goal.and_then(|sector| sector.arena_index()),
                recorded_goal_index,
                "{label}"
            );
            assert_eq!(replayed_layer, 0, "{label}");
        }
    }

    #[test]
    fn recording_drop_ale_keeps_click_titbit_distinct_from_authorized_seek_center() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id, _, _) = setup_drop_ale_sector_identity_scene();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        let click = crate::coordinates::MapPoint::new(80.0, 92.0);
        engine.world.fast_grid_mut().add_line(
            crate::fast_find_grid::GridLine::new(
                crate::coordinates::MapPoint::new(0.0, 90.0),
                crate::coordinates::MapPoint::new(127.0, 90.0),
                true,
            ),
            0,
        );
        let expected_titbit_position = engine.world.fast_grid.convert_2d_to_3d(
            click,
            crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
            engine.sight_obstacles(&assets),
        );
        let (expected_seek_center, _, _) = engine
            .resolve_drop_ale_target(pc_id, click, false, None, None)
            .expect("motion-line fixture must authorize a shifted Ale move box");
        assert_ne!(expected_seek_center, click);

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::DropAleAt {
                actor: pc_id,
                target_pos: click,
                running: false,
                already_authorized: false,
                goal_override: None,
                goal_sector_index_override: None,
                recorded_gate_path: None,
            },
        );

        let slot = engine
            .players
            .macro_store
            .get(pc_id)
            .and_then(|state| state.slot(0))
            .expect("DropAle recording occupies slot zero");
        assert_eq!(slot.steps[0].position, click);
        let QaReplayCommand::DropAle { target_pos, .. } = &slot.steps[0].replay else {
            panic!("recorded step must be DropAle")
        };
        assert_eq!(*target_pos, expected_seek_center);
        let titbit = engine
            .feedback
            .titbit_manager
            .titbits()
            .iter()
            .find(|titbit| titbit.kind == crate::titbit::TitbitKind::QuickAction)
            .expect("DropAle recording installs its QA titbit");
        assert_eq!(titbit.position, expected_titbit_position);
        assert_eq!(titbit.layer, 0);
    }

    #[test]
    fn recording_drop_ale_rejects_original_forbidden_target_sectors_without_stopping() {
        use crate::sector::{LiftType, SectorType};

        for (label, sector_type, lift_type) in [
            (
                "door",
                SectorType::MOTION | SectorType::AREA | SectorType::MOUSE | SectorType::DOOR,
                None,
            ),
            (
                "wall-ladder lift",
                SectorType::MOTION | SectorType::AREA | SectorType::MOUSE | SectorType::LIFT,
                Some(LiftType::Ladder),
            ),
        ] {
            let sim = crate::sim_rng::test_context();
            let (mut engine, assets, pc_id, source_index, _) =
                setup_drop_ale_sector_identity_scene();
            let sector = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level)
                .sectors
                .get_mut(usize::from(source_index))
                .expect("source sector");
            sector.sector_type = sector_type;
            sector.lift_type = lift_type;
            let mut display = HostDisplayState::default();
            let mut input = InputState::default();

            engine.apply_command(
                &sim,
                &mut display,
                &mut input,
                &assets,
                &PlayerCommand::StartRecordingMacro {
                    pc: Some(pc_id),
                    slot: 0,
                },
            );
            engine.apply_command(
                &sim,
                &mut display,
                &mut input,
                &assets,
                &PlayerCommand::DropAleAt {
                    actor: pc_id,
                    target_pos: crate::coordinates::MapPoint::new(80.0, 90.0),
                    running: false,
                    already_authorized: false,
                    goal_override: None,
                    goal_sector_index_override: None,
                    recorded_gate_path: None,
                },
            );

            assert!(engine.is_recording_macro(), "{label}");
            assert!(
                engine
                    .players
                    .macro_store
                    .get(pc_id)
                    .and_then(|state| state.slot(0))
                    .is_none_or(|slot| slot.steps.is_empty()),
                "{label}"
            );
            assert_eq!(
                engine.orders.sequence_manager.sequence_count(),
                0,
                "{label}"
            );
            assert!(
                engine.feedback.titbit_manager.titbits().is_empty(),
                "{label}"
            );
        }
    }

    #[test]
    fn recording_resolved_drop_ale_is_not_launched_live_and_replays_exact_route() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id, source_index, goal_index) =
            setup_drop_ale_sector_identity_scene();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        let authorized = crate::coordinates::MapPoint::new(180.0, 90.0);
        let goal = Some((crate::sector::SectorNumber::new(0), 0));
        let route = crate::gate::RecordedGatePath {
            source_sector: crate::sector::SectorNumber::new(0),
            source_sector_index: Some(source_index),
            source_layer: 0,
            outcome: crate::gate::RecordedGateOutcome::Failure,
        };

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::DropAleAt {
                actor: pc_id,
                target_pos: authorized,
                running: true,
                already_authorized: true,
                goal_override: goal,
                goal_sector_index_override: Some(goal_index),
                recorded_gate_path: Some(route.clone()),
            },
        );

        assert!(!engine.is_recording_macro());
        assert_eq!(
            engine.orders.sequence_manager.sequence_count(),
            0,
            "ale-input processing stores the sequence while recording instead of launching it"
        );
        let slot = engine
            .players
            .macro_store
            .get(pc_id)
            .and_then(|state| state.slot(0))
            .expect("DropAle recording occupies slot zero");
        assert_eq!(slot.steps.len(), 1);
        assert_eq!(
            slot.steps[0].replay,
            QaReplayCommand::DropAle {
                target_pos: authorized,
                running: true,
                already_authorized: true,
                goal_override: goal,
                goal_sector_index_override: Some(goal_index),
                recorded_gate_path: Some(route.clone()),
            }
        );

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );

        assert_eq!(
            drop_ale_seek_goal(&engine),
            (
                authorized,
                crate::position_interface::SectorHandle::new(0)
                    .map(|sector| sector.with_arena_index(goal_index)),
                0,
            ),
            "macro replay must retain the exact sparse goal-sector identity"
        );
        let replayed_route = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .and_then(|sequence| sequence.elements.first())
            .and_then(|element| element.recorded_gate_path.as_ref());
        assert_eq!(replayed_route, Some(&route));
    }

    #[test]
    fn point_seek_expansion_compares_goal_after_dispatch_time_door_adaptation() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        engine.scripts.mission = Some(minimal_script());
        let raw_goal = crate::position_interface::SectorHandle::new(22).unwrap();
        {
            let pc = engine.get_entity_mut(pc_id).unwrap();
            pc.element_data_mut().set_sector(Some(raw_goal));
            pc.element_data_mut().set_layer(2);
            pc.position_iface_mut().set_door(
                crate::position_interface::DoorHandle::new(7).expect("valid door index"),
                true,
            );
        }
        engine.script_domains.interactables.doors =
            (0..8).map(|_| crate::gate::Door::default()).collect();
        engine.script_domains.interactables.doors[7] = crate::gate::Door {
            active: true,
            sector_in: crate::sector::SectorNumber::new(133),
            layer_in: 11,
            sector_out: crate::sector::SectorNumber::new(22),
            layer_out: 2,
            ..crate::gate::Door::default()
        };
        let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
        let mut seek = SequenceElement::new_movement(
            1,
            Command::Seek,
            Some(pc_id),
            crate::order::OrderType::WalkingUpright,
        );
        seek.recorded_gate_path = Some(crate::gate::RecordedGatePath {
            source_sector: crate::sector::SectorNumber::new(133),
            source_sector_index: None,
            source_layer: 11,
            outcome: crate::gate::RecordedGateOutcome::Failure,
        });
        let sequence_id = engine.orders.sequence_manager.launch_element(seek);

        assert!(engine.try_dispatch_cross_sector_point_seek(
            &crate::sim_rng::test_context(),
            &assets,
            pc_id,
            sequence_id,
            0,
            destination,
            Some(raw_goal),
            2,
            crate::order::OrderType::WalkingUpright,
            crate::sequence::MoveFlags::SEEK,
            0.0,
            Some(crate::gate::RecordedGatePath {
                source_sector: crate::sector::SectorNumber::new(133),
                source_sector_index: None,
                source_layer: 11,
                outcome: crate::gate::RecordedGateOutcome::Failure,
            }),
            crate::sequence::PointSeekRouteProvenance::OriginalReplay,
        ));
    }

    #[test]
    #[should_panic(expected = "public source sector differs at dispatch")]
    fn point_seek_expansion_validates_recorded_source_before_adapted_same_sector_return() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        engine.scripts.mission = Some(minimal_script());
        let raw_goal = crate::position_interface::SectorHandle::new(22).unwrap();
        {
            let pc = engine.get_entity_mut(pc_id).unwrap();
            pc.element_data_mut().set_sector(Some(raw_goal));
            pc.element_data_mut().set_layer(2);
            pc.position_iface_mut().set_door(
                crate::position_interface::DoorHandle::new(7).expect("valid door index"),
                false,
            );
        }
        engine.script_domains.interactables.doors =
            (0..8).map(|_| crate::gate::Door::default()).collect();
        engine.script_domains.interactables.doors[7] = crate::gate::Door {
            active: true,
            sector_in: crate::sector::SectorNumber::new(133),
            layer_in: 11,
            sector_out: crate::sector::SectorNumber::new(22),
            layer_out: 2,
            ..crate::gate::Door::default()
        };
        let destination = crate::coordinates::MapPoint::new(778.0, 1714.0);
        let sequence_id =
            engine
                .orders
                .sequence_manager
                .launch_element(SequenceElement::new_movement(
                    1,
                    Command::Seek,
                    Some(pc_id),
                    crate::order::OrderType::WalkingUpright,
                ));

        engine.try_dispatch_cross_sector_point_seek(
            &crate::sim_rng::test_context(),
            &assets,
            pc_id,
            sequence_id,
            0,
            destination,
            Some(raw_goal),
            2,
            crate::order::OrderType::WalkingUpright,
            crate::sequence::MoveFlags::SEEK,
            0.0,
            Some(crate::gate::RecordedGatePath {
                source_sector: crate::sector::SectorNumber::new(133),
                source_sector_index: None,
                source_layer: 11,
                outcome: crate::gate::RecordedGateOutcome::Failure,
            }),
            crate::sequence::PointSeekRouteProvenance::OriginalReplay,
        );
    }

    #[test]
    fn resolved_replay_drop_ale_without_exact_index_keeps_legacy_number_only_goal() {
        let (mut engine, assets, pc_id, _, _) = setup_drop_ale_sector_identity_scene();
        let authorized = crate::coordinates::MapPoint::new(180.0, 90.0);

        engine.apply_command(
            &crate::sim_rng::test_context(),
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::DropAleAt {
                actor: pc_id,
                target_pos: authorized,
                running: false,
                already_authorized: true,
                goal_override: Some((crate::sector::SectorNumber::new(0), 0)),
                goal_sector_index_override: None,
                recorded_gate_path: None,
            },
        );

        assert_eq!(
            drop_ale_seek_goal(&engine),
            (
                authorized,
                crate::position_interface::SectorHandle::new(0),
                0,
            )
        );
    }

    #[test]
    #[should_panic(expected = "outside the FastFindGrid sector table")]
    fn resolved_replay_drop_ale_rejects_out_of_range_exact_index() {
        let (mut engine, _, pc_id, _, _) = setup_drop_ale_sector_identity_scene();
        engine.apply_drop_ale_at(
            pc_id,
            crate::coordinates::MapPoint::new(180.0, 90.0),
            false,
            true,
            Some((crate::sector::SectorNumber::new(0), 0)),
            crate::fast_find_grid::SectorIndex::new(9999),
            None,
        );
    }

    #[test]
    #[should_panic(expected = "has public sector")]
    fn resolved_replay_drop_ale_rejects_disagreeing_exact_index() {
        let (mut engine, _, pc_id, _, goal_index) = setup_drop_ale_sector_identity_scene();
        std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level).sectors
            [usize::from(goal_index)]
        .sector_number = crate::sector::SectorNumber::new(1);
        engine.apply_drop_ale_at(
            pc_id,
            crate::coordinates::MapPoint::new(180.0, 90.0),
            false,
            true,
            Some((crate::sector::SectorNumber::new(0), 0)),
            Some(goal_index),
            None,
        );
    }

    #[test]
    #[should_panic(expected = "exact goal-sector identity requires a goal_override")]
    fn drop_ale_rejects_exact_index_without_goal_override() {
        let (mut engine, _, pc_id, _, goal_index) = setup_drop_ale_sector_identity_scene();
        engine.apply_drop_ale_at(
            pc_id,
            crate::coordinates::MapPoint::new(180.0, 90.0),
            false,
            false,
            None,
            Some(goal_index),
            None,
        );
    }

    #[test]
    #[should_panic(expected = "goal_override has invalid public sector")]
    fn resolved_replay_drop_ale_rejects_invalid_public_sector() {
        let (mut engine, _, pc_id, _, _) = setup_drop_ale_sector_identity_scene();
        engine.apply_drop_ale_at(
            pc_id,
            crate::coordinates::MapPoint::new(180.0, 90.0),
            false,
            true,
            Some((crate::sector::SectorNumber::new(-1), 0)),
            None,
            None,
        );
    }

    fn setup_strangle_command_scene() -> (EngineInner, LevelAssets, EntityId, EntityId) {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Strangle, 0)]);
        let sector = crate::position_interface::SectorHandle::new(1);
        {
            let pc = engine.get_entity_mut(pc_id).expect("test PC exists");
            pc.element_data_mut()
                .set_position_map(crate::coordinates::MapPoint::new(100.0, 100.0));
            pc.element_data_mut().set_sector(sector);
            pc.pc_data_mut().expect("test PC data").current_action = Action::Strangle;
        }
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::Strangling,
            crate::coordinates::SpriteLocalPoint::new(30.0, 0.0),
            crate::coordinates::SpriteAnchor::new(0.0, 0.0),
        );

        let mut target = ActorSoldier {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData {
                cached_camp: Camp::Lacklandists,
                ..SoldierData::default()
            },
        };
        target
            .element
            .set_position_map(crate::coordinates::MapPoint::new(110.0, 100.0));
        target.element.set_sector(sector);
        let target_id = engine.add_entity(Entity::Soldier(target));

        (engine, assets, pc_id, target_id)
    }

    fn assert_single_recorded_titbit(
        engine: &EngineInner,
        actor: EntityId,
        supplier: Option<EntityId>,
        phase: crate::titbit::QuickAction,
        position: crate::coordinates::WorldPoint3D,
        layer: u16,
        running: bool,
    ) {
        let manager = &engine.feedback.titbit_manager;
        assert_eq!(
            manager.parity_current_id(),
            1,
            "one titbit insertion must advance current_id exactly once"
        );
        assert_eq!(manager.titbits().len(), if running { 2 } else { 1 });
        let titbit = &manager.titbits()[0];
        assert_eq!(titbit.id, crate::titbit::TitbitId::new(0).unwrap());
        assert_eq!(titbit.kind, crate::titbit::TitbitKind::QuickAction);
        assert_eq!(titbit.phase, phase as u16);
        assert_eq!(
            titbit.sprite_row,
            crate::titbit::SpriteRow::QuickActionTitbits as u16
        );
        assert_eq!(titbit.sprite_frame, 0);
        assert_eq!(titbit.frame_count, 0);
        assert!(!titbit.blinking);
        assert_eq!(
            titbit.element_supplier,
            supplier.map(|id| crate::titbit::ElementHandle(id.index()))
        );
        assert_eq!(
            titbit.element_manager,
            Some(crate::titbit::ElementHandle(actor.index()))
        );
        assert_eq!(titbit.position, position);
        assert_eq!(titbit.layer, layer);
        let expected_display_y = if let Some(supplier) = supplier {
            engine
                .get_entity(supplier)
                .map(|entity| entity.element_data().position_map().y)
                .expect("titbit supplier remains live")
        } else {
            position.y
        };
        assert_eq!(titbit.display_order, expected_display_y + 0.01);
        assert_eq!(
            engine
                .players
                .macro_store
                .get(actor)
                .and_then(|state| state.get_slot_titbit(0))
                .map(crate::titbit::TitbitId::get),
            Some(0),
            "the QA slot must retain the sole allocated titbit id"
        );
        if running {
            let run = &manager.titbits()[1];
            assert_eq!(run.id, titbit.id);
            assert_eq!(run.kind, crate::titbit::TitbitKind::QuickActionRun);
        }
    }

    #[test]
    fn recording_strangle_stores_macro_without_launching_live_interaction() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id, target_id) = setup_strangle_command_scene();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target: target_id,
                command: Command::StrangleCmd,
                running: false,
            },
        );

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
        assert!(!engine.is_recording_macro());
        let state = engine
            .players
            .macro_store
            .get(pc_id)
            .expect("recording PC has macro state");
        let slot = state.slot(0).expect("Strangle was stored in slot zero");
        assert_eq!(slot.steps.len(), 1);
        assert_eq!(slot.steps[0].action, Action::Strangle);
        assert_eq!(
            slot.steps[0].replay,
            QaReplayCommand::Interaction {
                target: target_id,
                command: Command::StrangleCmd,
                double_click: false,
            }
        );
        assert!(state.get_slot_titbit(0).is_some());
        assert_single_recorded_titbit(
            &engine,
            pc_id,
            Some(target_id),
            crate::titbit::QuickAction::Strangle,
            crate::coordinates::WorldPoint3D::ZERO,
            0,
            false,
        );

        // Playback happens after recording has stopped and must take the live
        // route rather than being suppressed by the recording-only guard.
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);
    }

    #[test]
    fn recording_running_strangle_marks_replacement_titbit_as_running() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id, target_id) = setup_strangle_command_scene();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target: target_id,
                command: Command::StrangleCmd,
                running: true,
            },
        );

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
        assert!(!engine.is_recording_macro());
        let state = engine
            .players
            .macro_store
            .get(pc_id)
            .expect("recording PC has macro state");
        let slot = state.slot(0).expect("running Strangle occupies slot zero");
        assert_eq!(
            slot.steps[0].replay,
            QaReplayCommand::Interaction {
                target: target_id,
                command: Command::StrangleCmd,
                double_click: true,
            }
        );
        let titbit_id = state
            .get_slot_titbit(0)
            .expect("running Strangle records a replacement titbit");
        assert!(engine.feedback.titbit_manager.is_running_for_qa(titbit_id));
    }

    #[test]
    fn resolved_orientation_restores_the_implicit_messenger_action() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id, _) = setup_strangle_command_scene();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        assert_eq!(engine.get_selected_action(), Action::NoAction);
        let previous_pc_action = engine
            .get_entity(pc_id)
            .unwrap()
            .pc_data()
            .unwrap()
            .current_action;

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::PerformResolvedOrientation {
                pc_id,
                action: Action::Net,
                mouse_map: crate::coordinates::MapPoint::new(200.0, 200.0),
                target: crate::coordinates::WorldPoint3D::new(200.0, 200.0, 0.0),
            },
        );

        assert_eq!(engine.get_selected_action(), Action::Net);
        assert_eq!(
            engine
                .get_entity(pc_id)
                .unwrap()
                .pc_data()
                .unwrap()
                .current_action,
            previous_pc_action,
            "the orientation attests messenger state, not a player-character action mutation"
        );
    }

    #[test]
    fn recorded_native_interactions_use_their_original_authored_titbit_metadata() {
        let cases = [
            (Command::ShootBow, Action::Stone, QuickAction::BowOk),
            (
                Command::TakeCorpse,
                Action::LittleJohnCarry,
                QuickAction::Walk,
            ),
            (Command::ClimbUpOnShoulders, Action::Jump, QuickAction::Walk),
            (Command::Untie, Action::Tie, QuickAction::Tie),
        ];

        for (command, selected_action, expected_phase) in cases {
            let sim = crate::sim_rng::test_context();
            let (mut engine, assets, pc_id, target_id) = setup_strangle_command_scene();
            engine
                .get_entity_mut(pc_id)
                .expect("recording PC")
                .pc_data_mut()
                .expect("recording PC data")
                .current_action = selected_action;
            engine
                .get_entity_mut(target_id)
                .expect("recorded target")
                .element_data_mut()
                .set_layer(7);
            let mut display = HostDisplayState::default();
            let mut input = InputState::default();

            engine.apply_command(
                &sim,
                &mut display,
                &mut input,
                &assets,
                &PlayerCommand::StartRecordingMacro {
                    pc: Some(pc_id),
                    slot: 0,
                },
            );
            engine.apply_command(
                &sim,
                &mut display,
                &mut input,
                &assets,
                &PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target: target_id,
                    command,
                    running: false,
                },
            );

            assert_single_recorded_titbit(
                &engine,
                pc_id,
                Some(target_id),
                expected_phase,
                WorldPoint3D::ZERO,
                7,
                false,
            );
        }
    }

    #[test]
    fn recorded_ground_throws_keep_their_original_layer_and_supplier_metadata() {
        let cases = [
            (
                Action::Stone,
                Command::ThrowStone,
                Field::NoiseDistractionTarget,
                QuickAction::Stone,
                9,
                0,
            ),
            (
                Action::Purse,
                Command::ThrowPurse,
                Field::PurseTarget,
                QuickAction::Purse,
                9,
                0,
            ),
            (
                Action::Net,
                Command::ThrowNet,
                Field::NetTarget,
                QuickAction::Net,
                9,
                0,
            ),
            (
                Action::WaspNest,
                Command::ThrowWaspNest,
                Field::WaspNestTarget,
                QuickAction::Wasp,
                9,
                9,
            ),
        ];

        for (action, command, target_field, expected_phase, captured_layer, expected_layer) in cases
        {
            let sim = crate::sim_rng::test_context();
            let (mut engine, assets, pc_id) = setup_pc_engine(&[(action, 1)]);
            engine
                .get_entity_mut(pc_id)
                .expect("recording PC")
                .pc_data_mut()
                .expect("recording PC data")
                .current_action = action;
            let mut display = HostDisplayState::default();
            let mut input = InputState::default();

            engine.apply_command(
                &sim,
                &mut display,
                &mut input,
                &assets,
                &PlayerCommand::StartRecordingMacro {
                    pc: Some(pc_id),
                    slot: 0,
                },
            );
            let target_pos = WorldPoint3D::new(30.0, 40.0, 5.0);
            engine.apply_command(
                &sim,
                &mut display,
                &mut input,
                &assets,
                &PlayerCommand::LaunchGroundTarget {
                    actor: pc_id,
                    target_pos,
                    command,
                    target_field,
                    titbit_layer: captured_layer,
                },
            );

            assert_single_recorded_titbit(
                &engine,
                pc_id,
                None,
                expected_phase,
                target_pos,
                expected_layer,
                false,
            );
        }
    }

    #[test]
    fn recording_ground_target_allocates_one_original_faithful_titbit() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::WaspNest, 1)]);
        engine
            .get_entity_mut(pc_id)
            .and_then(Entity::pc_data_mut)
            .expect("test PC data")
            .current_action = Action::WaspNest;
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        let target = crate::coordinates::WorldPoint3D::new(25.0, 40.0, 7.0);

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchGroundTarget {
                actor: pc_id,
                target_pos: target,
                command: Command::ThrowWaspNest,
                target_field: Field::WaspNestTarget,
                titbit_layer: 9,
            },
        );

        assert!(!engine.is_recording_macro());
        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
        let step = &engine
            .players
            .macro_store
            .get(pc_id)
            .and_then(|state| state.slot(0))
            .expect("ground target was stored in slot zero")
            .steps[0];
        assert_eq!(
            step.replay,
            QaReplayCommand::GroundTarget {
                target_pos: target,
                command: Command::ThrowWaspNest,
                target_field: Field::WaspNestTarget,
                titbit_layer: 9,
            }
        );
        assert_single_recorded_titbit(
            &engine,
            pc_id,
            None,
            crate::titbit::QuickAction::Wasp,
            target,
            9,
            false,
        );
    }

    #[test]
    fn recorded_running_interaction_replays_one_running_seek_route() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id, target_id) = setup_strangle_command_scene();
        engine
            .get_entity_mut(target_id)
            .expect("Strangle target exists")
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(200.0, 100.0));
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target: target_id,
                command: Command::StrangleCmd,
                running: true,
            },
        );

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );

        assert_eq!(
            engine.orders.sequence_manager.sequence_count(),
            1,
            "quick-action startup clones the recorded route once"
        );
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("running interaction launches its stored route");
        let seek = sequence.get(0).expect("interaction route begins with Seek");
        assert_eq!(seek.command, Command::Seek);
        let SequenceElementData::Movement {
            action,
            element,
            post_seek_sequence,
            ..
        } = &seek.data
        else {
            panic!("running interaction route must begin with movement");
        };
        assert_eq!(*action, crate::order::OrderType::RunningUpright);
        assert_eq!(*element, Some(target_id));
        let post_seek = post_seek_sequence
            .as_ref()
            .expect("recorded route retains its interaction continuation");
        assert_eq!(post_seek.len(), 1);
        assert_eq!(post_seek.get(0).unwrap().command, Command::StrangleCmd);
    }

    #[test]
    #[should_panic(expected = "recorded interaction target")]
    fn recording_interaction_panics_when_target_is_missing() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id, _target_id) = setup_strangle_command_scene();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        let missing_target = EntityId::Soldier(crate::entity_id::SoldierId(u32::MAX));
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target: missing_target,
                command: Command::StrangleCmd,
                running: false,
            },
        );
    }

    #[test]
    fn missing_recording_target_preflight_is_read_only() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id, _target_id) = setup_strangle_command_scene();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        let missing_target = EntityId::Soldier(crate::entity_id::SoldierId(u32::MAX));
        let recording_before = engine.is_recording_macro();
        let sequence_count_before = engine.orders.sequence_manager.sequence_count();
        let (slot_before, titbit_before) = {
            let state = engine
                .players
                .macro_store
                .get(pc_id)
                .expect("recording PC has macro state");
            (state.slot(0).cloned(), state.get_slot_titbit(0))
        };

        assert_eq!(
            engine.validate_recorded_interaction_identities(pc_id, missing_target),
            Err(RecordedInteractionIdentityError::MissingTarget)
        );
        assert_eq!(engine.is_recording_macro(), recording_before);
        assert_eq!(
            engine.orders.sequence_manager.sequence_count(),
            sequence_count_before
        );
        let state = engine
            .players
            .macro_store
            .get(pc_id)
            .expect("recording PC has macro state");
        assert_eq!(state.slot(0), slot_before.as_ref());
        assert_eq!(state.get_slot_titbit(0), titbit_before);
    }

    #[test]
    fn live_strangle_still_launches_interaction_when_not_recording() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id, target_id) = setup_strangle_command_scene();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target: target_id,
                command: Command::StrangleCmd,
                running: false,
            },
        );

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("live Strangle launches a sequence");
        assert_eq!(sequence.len(), 1);
        let seek = sequence.get(0).expect("live Strangle route has a seek");
        assert_eq!(seek.command, Command::Seek);
        let SequenceElementData::Movement {
            post_seek_sequence, ..
        } = &seek.data
        else {
            panic!("live Strangle route must begin with movement");
        };
        let post_seek = post_seek_sequence
            .as_ref()
            .expect("live Strangle seek retains its interaction");
        assert_eq!(post_seek.len(), 1);
        assert_eq!(post_seek.get(0).unwrap().command, Command::StrangleCmd);
    }

    fn spawn_pc_at(engine: &mut EngineInner, x: f32, y: f32) -> EntityId {
        let mut pc = ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        pc.element
            .set_position_map(crate::coordinates::MapPoint { x, y });
        engine.add_entity(Entity::Pc(pc))
    }

    fn spawn_friendly_civilian(engine: &mut EngineInner) -> EntityId {
        let mut civilian = ActorCivilian {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorCivilian;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            civilian: crate::element::CivilianData {
                cached_civilian_type: crate::profiles::CivilianType::Beggar,
                ..Default::default()
            },
        };
        civilian.npc.ai_brain = crate::element::AiBrain::Friendly(Box::default());
        engine.add_entity(Entity::Civilian(civilian))
    }

    fn friendly_beggar_dont_talk_counter(engine: &EngineInner, target: EntityId) -> u16 {
        let Some(Entity::Civilian(civilian)) = engine.get_entity(target) else {
            panic!("friendly counter target is not a civilian");
        };
        let crate::element::AiBrain::Friendly(ai) = &civilian.npc.ai_brain else {
            panic!("friendly counter target does not have FriendlyAi");
        };
        ai.beggar_dont_talk_counter
    }

    fn first_seek_tolerance(engine: &EngineInner) -> f32 {
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .unwrap();
        let seek = sequence.get(0).unwrap();
        match &seek.data {
            SequenceElementData::Movement { tolerance, .. } => *tolerance,
            other => panic!("expected movement seek element, got {other:?}"),
        }
    }

    #[test]
    fn sword_strike_seek_uses_resolved_tolerance_and_authored_sword_movement() {
        let (mut engine, mut assets, pc_id) = setup_pc_engine(&[]);
        {
            let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
            profiles.characters[0].hth_weapon_id = 1;
            let mut weapon = crate::profiles::HtHWeaponProfile::default();
            weapon.thrusts[crate::weapons::SwordStrike::D as usize].maximal_distance = 60;
            profiles.hth_weapons.push(weapon);
        }

        let sector = crate::position_interface::SectorHandle::new(0);
        engine
            .get_entity_mut(pc_id)
            .expect("test PC exists")
            .element_data_mut()
            .set_sector(sector);
        let mut target = ActorCivilian {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorCivilian;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            civilian: Default::default(),
        };
        target.element.set_sector(sector);
        let target_id = engine.add_entity(Entity::Civilian(target));

        engine.apply_sword_strike_with_seek(
            &assets,
            pc_id,
            target_id,
            Command::SwordstrikeThrustD,
            None,
            GestureQuality::PERFECT,
            Some(63.0),
        );

        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("strike seek sequence was launched");
        assert_eq!(sequence.len(), 1);
        let seek = sequence.get(0).expect("seek element exists");
        assert_eq!(seek.command, Command::Seek);
        let SequenceElementData::Movement {
            action,
            element,
            tolerance,
            flags,
            post_seek_sequence,
            ..
        } = &seek.data
        else {
            panic!("strike seek must be a movement element");
        };
        assert_eq!(*action, crate::order::OrderType::RunningWithSword);
        assert_eq!(*element, Some(target_id));
        assert_eq!(*tolerance, 63.0);
        assert!(flags.contains(MoveFlags::SEEK));
        assert!(!flags.contains(MoveFlags::FORCE_SWORD_MOVEMENT));

        let post_seek = post_seek_sequence
            .as_ref()
            .expect("strike seek retains its post-seek strike");
        assert_eq!(post_seek.len(), 1);
        let strike = post_seek.get(0).expect("post-seek strike exists");
        assert_eq!(strike.command, Command::SwordstrikeThrustD);
        assert_eq!(strike.owner, Some(pc_id));
        assert!(matches!(
            &strike.data,
            SequenceElementData::Interaction {
                antagonist: Some(id)
            } if *id == target_id
        ));
    }

    #[test]
    fn composite_seek_retains_both_strikes_and_quality() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let target_id = spawn_pc_at(&mut engine, 90.0, 10.0);
        engine.apply_sword_strike_with_seek(
            &assets,
            pc_id,
            target_id,
            Command::SwordstrikeThrustD,
            Some(CompositeSwordTechnique::RisingFeint),
            GestureQuality::FAIR,
            Some(63.0),
        );

        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("strike seek sequence");
        let SequenceElementData::Movement {
            post_seek_sequence: Some(post_seek),
            ..
        } = &sequence.elements[0].data
        else {
            panic!("composite seek lost post-seek sequence");
        };
        assert_eq!(post_seek.elements.len(), 2);
        assert_eq!(post_seek.elements[0].command, Command::SwordstrikeThrustD);
        assert_eq!(post_seek.elements[1].command, Command::SwordstrikeThrustB);
        assert!(
            post_seek
                .elements
                .iter()
                .all(|element| element.gesture_quality == GestureQuality::FAIR)
        );
    }

    #[test]
    fn sword_strike_seek_treats_two_unassigned_sectors_as_same_like_original() {
        let (mut engine, mut assets, pc_id) = setup_pc_engine(&[]);
        {
            let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
            profiles.characters[0].hth_weapon_id = 1;
            profiles
                .hth_weapons
                .push(crate::profiles::HtHWeaponProfile::default());
        }
        assert_eq!(
            engine.get_entity(pc_id).unwrap().element_data().sector(),
            None
        );
        let target_id = engine.add_entity(Entity::Civilian(ActorCivilian {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorCivilian;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            civilian: Default::default(),
        }));

        engine.apply_sword_strike_with_seek(
            &assets,
            pc_id,
            target_id,
            Command::SwordstrikeThrustA,
            None,
            GestureQuality::PERFECT,
            Some(63.0),
        );
        assert_eq!(first_seek_tolerance(&engine), 63.0);
    }

    #[test]
    fn cross_gate_swordfight_preserves_entity_seek_refresh_and_post_seek_entry() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        engine.scripts.mission = Some(minimal_script());

        let pc_sector = crate::position_interface::SectorHandle::new(7);
        let target_sector = crate::position_interface::SectorHandle::new(8);
        {
            let pc_entity = engine.get_entity_mut(pc_id).expect("test PC exists");
            pc_entity
                .position_iface_mut()
                .set_move_box(crate::coordinates::MoveBox::from_coords(
                    -4.0, -4.0, 4.0, 4.0,
                ));
            let pc = pc_entity.element_data_mut();
            pc.set_position_map(crate::coordinates::MapPoint::new(10.0, 30.0));
            pc.set_sector(pc_sector);
        }

        let mut target = ActorSoldier {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            soldier: SoldierData {
                cached_camp: Camp::Lacklandists,
                ..SoldierData::default()
            },
        };
        target.element.sprite.position_iface.set_move_box(
            crate::coordinates::MoveBox::from_coords(-4.0, -4.0, 4.0, 4.0),
        );
        target
            .element
            .set_position_map(crate::coordinates::MapPoint::new(90.0, 30.0));
        target.element.set_sector(target_sector);
        let target_id = engine.add_entity(Entity::Soldier(target));

        engine
            .script_domains
            .interactables
            .doors
            .push(crate::gate::Door {
                point_out: crate::coordinates::MapPoint::new(30.0, 30.0),
                point_mid: crate::coordinates::MapPoint::new(40.0, 30.0),
                point_in: crate::coordinates::MapPoint::new(50.0, 30.0),
                sector_out: crate::sector::SectorNumber::new(7),
                sector_in: crate::sector::SectorNumber::new(8),
                ..crate::gate::Door::default()
            });

        engine.apply_enter_swordfight(&sim, &assets, pc_id, target_id, false);
        engine.hourglass_phase_sequences(&sim, &mut HostDisplayState::default(), &assets);

        let route = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .find(|sequence| {
                sequence
                    .elements
                    .iter()
                    .any(|element| element.command == Command::PassDoor)
            })
            .expect("cross-gate swordfight route was launched");
        let commands: Vec<_> = route
            .elements
            .iter()
            .map(|element| element.command)
            .collect();
        assert!(!commands.contains(&Command::EnterSwordfight));
        assert!(!commands.contains(&Command::SpeakHeroReachDestination));
        assert!(!commands.contains(&Command::EquipBow));

        let approach = route
            .elements
            .iter()
            .find(|element| element.command == Command::Move)
            .expect("gate route starts with a movement approach");
        let SequenceElementData::Movement { element, .. } = &approach.data else {
            panic!("gate approach must remain a movement element");
        };
        assert_eq!(*element, Some(target_id));

        let actor = engine
            .get_entity(pc_id)
            .and_then(|entity| entity.actor_data())
            .expect("test PC has actor state");
        assert_eq!(actor.wait_time, 25);
        assert_eq!(actor.seek_refresh_wait, 25);
        assert_eq!(actor.seek_target, Some(target_id));
        assert_eq!(actor.seek_distance, 40.0);
        let post_seek = actor
            .post_seek_sequence
            .as_ref()
            .expect("EnterSwordfight remains owned by the active entity seek");
        assert_eq!(post_seek.len(), 1);
        let enter = post_seek.get(0).expect("post-seek swordfight entry exists");
        assert_eq!(enter.command, Command::EnterSwordfight);
        assert!(matches!(
            enter.get_property(Field::Opponent),
            Some(FieldValue::Element(id)) if *id == target_id
        ));
    }

    #[test]
    fn newer_strike_seek_replaces_old_preference_behind_injury() {
        use crate::sequence::{SequencePriority, SequenceState};

        let (mut engine, mut assets, pc_id) = setup_pc_engine(&[]);
        {
            let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
            profiles.characters[0].hth_weapon_id = 1;
            let mut weapon = crate::profiles::HtHWeaponProfile::default();
            weapon.thrusts[crate::weapons::SwordStrike::E as usize].maximal_distance = 60;
            profiles.hth_weapons.push(weapon);
        }
        let sector = crate::position_interface::SectorHandle::new(0);
        engine
            .get_entity_mut(pc_id)
            .unwrap()
            .element_data_mut()
            .set_sector(sector);
        let mut target = ActorCivilian {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorCivilian;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            civilian: Default::default(),
        };
        target.element.set_sector(sector);
        let target_id = engine.add_entity(Entity::Civilian(target));

        let mut injury = SequenceElement::new(1, Command::ReceiveSwordDamage, Some(pc_id));
        injury.priority = SequencePriority::Injury;
        let injury_seq = engine.orders.sequence_manager.launch_element(injury);
        engine
            .orders
            .sequence_manager
            .element_in_progress(injury_seq, 0);

        let mut old_strike = SequenceElement::new_interaction(
            1,
            Command::SwordstrikeThrustD,
            Some(pc_id),
            Some(target_id),
        );
        old_strike.priority = SequencePriority::Preference;
        let old_strike_seq = engine.orders.sequence_manager.launch_element(old_strike);
        engine.engine_postpone(injury_seq, 0, old_strike_seq, 0);

        engine.apply_sword_strike_with_seek(
            &assets,
            pc_id,
            target_id,
            Command::SwordstrikeThrustE,
            None,
            GestureQuality::PERFECT,
            Some(54.0),
        );

        // Sequence-element admission: the newer seek is registered at
        // the manager tail without synchronous arbitration, so the older
        // postponed Preference strike keeps its slot behind the injury.
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(old_strike_seq, 0)
                .unwrap()
                .state,
            SequenceState::Postponed,
            "tail admission must not synchronously interrupt the older postponed strike"
        );
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(injury_seq, 0)
                .unwrap()
                .cross_postponed,
            Some((old_strike_seq, 0)),
            "the injury keeps its original postponed successor"
        );
        let (new_seek_seq, new_seek) = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .filter_map(|sequence| {
                let element = sequence.get(0)?;
                (element.command == Command::Seek).then_some((sequence.id, element))
            })
            .next()
            .expect("the newer strike seek was registered");
        assert_ne!(new_seek_seq, old_strike_seq);
        assert_eq!(
            new_seek.state,
            SequenceState::Todo,
            "the newer seek waits for the update instead of replacing the postponed chain"
        );
    }

    fn minimal_script() -> crate::engine::types::MissionScript {
        use crate::scb::{ClassEntry, Function, ScbFile};
        use crate::vm::{Opcode, Quad};

        let startup = ClassEntry {
            source_file: "test.scs".into(),
            class_name: "StartUp".into(),
            size_of_member_variables: 0,
            member_variables: Vec::new(),
            functions: vec![Function {
                name: "Initialize".into(),
                address: 0,
                num_parameters: 0,
                size_of_return_value: 0,
                size_of_parameters: 0,
                size_of_volatile: 0,
                size_of_temporary: 0,
            }],
            quads: vec![
                Quad {
                    operation: Opcode::BeginFunction as u8,
                    operands: [0; 8],
                },
                Quad {
                    operation: Opcode::Return as u8,
                    operands: [0; 8],
                },
            ],
        };
        MissionScript::from_scb(ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![startup],
        })
        .expect("minimal mission script builds")
    }

    fn setup_scroll_read_scene() -> (EngineInner, LevelAssets, EntityId, EntityId, EntityId) {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Search, 0)]);
        engine.scripts.mission = Some(minimal_script());
        {
            let pc = engine.get_entity_mut(pc_id).unwrap().element_data_mut();
            pc.set_position_map(crate::coordinates::MapPoint { x: 100.0, y: 100.0 });
            pc.set_direction_instantly(0);
        }
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::Listening,
            crate::coordinates::SpriteLocalPoint::new(30.0, 0.0),
            crate::coordinates::SpriteAnchor::new(0.0, 0.0),
        );

        let mut npc = ActorCivilian {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorCivilian;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                ai: crate::element::AiActorData {
                    attached_scroll: None,
                    ..Default::default()
                },
                ..NpcData::default()
            },
            civilian: Default::default(),
        };
        npc.element
            .set_position_map(crate::coordinates::MapPoint { x: 110.0, y: 100.0 });
        let npc_id = engine.add_entity(Entity::Civilian(npc));

        let scroll_id = spawn_scroll(&mut engine, true);
        match engine.get_entity_mut(npc_id) {
            Some(Entity::Civilian(civilian)) => {
                civilian.npc.attached_scroll = Some(scroll_id);
            }
            _ => unreachable!("newly spawned scroll-reader NPC changed kind"),
        }
        engine.script_domains.scrolls.attachments.insert(
            crate::natives::ScriptHandleCodec::actor_handle(npc_id),
            crate::natives::ScriptHandleCodec::actor_handle(scroll_id),
        );

        (engine, assets, pc_id, npc_id, scroll_id)
    }

    fn assert_scroll_read_composite<P: robin_util::state_hash::StateHash>(
        sequence: &Sequence<P>,
        pc_id: EntityId,
        npc_id: EntityId,
        scroll_id: EntityId,
    ) {
        assert_eq!(sequence.elements.len(), 5);
        assert_eq!(sequence.elements.first().unwrap().command, Command::LockAi);
        assert_eq!(sequence.elements.first().unwrap().owner, Some(npc_id));
        assert_eq!(
            sequence.elements.get(1).unwrap().command,
            Command::TurnElement
        );
        assert_eq!(sequence.elements.get(1).unwrap().owner, Some(pc_id));
        assert_eq!(
            sequence.elements.get(2).unwrap().command,
            Command::TurnElement
        );
        assert_eq!(sequence.elements.get(2).unwrap().owner, Some(npc_id));
        assert_eq!(sequence.elements.get(3).unwrap().command, Command::UnlockAi);
        assert_eq!(sequence.elements.get(3).unwrap().owner, Some(npc_id));

        let open = sequence.elements.get(4).unwrap();
        assert_eq!(open.command, Command::OpenScroll);
        assert_eq!(open.command_level, 2);
        let SequenceElementData::Generic { properties } = &open.data else {
            panic!("OpenScroll must carry generic properties");
        };
        assert!(matches!(
            properties.get(&Field::Scroll),
            Some(FieldValue::Element(id)) if *id == scroll_id
        ));
        assert!(matches!(
            properties.get(&Field::ScrollReader),
            Some(FieldValue::Element(id)) if *id == pc_id
        ));
        assert!(matches!(
            properties.get(&Field::ScrollOwner),
            Some(FieldValue::Element(id)) if *id == npc_id
        ));
    }

    #[test]
    fn scroll_read_recording_stores_semantic_step_and_does_not_launch_live_sequence() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let (mut engine, assets, pc_id, npc_id, _scroll_id) = setup_scroll_read_scene();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchScrollRead {
                actor: pc_id,
                target: npc_id,
                running: false,
            },
        );

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
        assert!(!engine.is_recording_macro());
        let state = engine
            .players
            .macro_store
            .get(pc_id)
            .expect("pc macro state");
        let slot = state.slot(0).expect("slot 0");
        assert_eq!(slot.steps.len(), 1);
        assert_eq!(
            slot.steps[0].replay,
            QaReplayCommand::ScrollRead {
                target: npc_id,
                running: false,
            }
        );
        assert_single_recorded_titbit(
            &engine,
            pc_id,
            Some(npc_id),
            crate::titbit::QuickAction::Speak,
            crate::coordinates::WorldPoint3D::ZERO,
            0,
            false,
        );
    }

    #[test]
    fn scroll_read_macro_replay_rebuilds_live_sequence_shape() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let (mut engine, assets, pc_id, npc_id, scroll_id) = setup_scroll_read_scene();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        let state = engine.players.macro_store.get_or_insert(pc_id);
        state.begin_recording(0);
        state.append_if_recording(QuickActionStep {
            action: Action::Search,
            position: crate::coordinates::MapPoint::new(110.0, 100.0),
            replay: QaReplayCommand::ScrollRead {
                target: npc_id,
                running: false,
            },
        });
        state.stop_recording();

        engine.apply_command(
            sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .unwrap();
        assert_scroll_read_composite(sequence, pc_id, npc_id, scroll_id);
    }

    #[test]
    fn recorded_running_scroll_read_replays_one_running_seek_route() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id, npc_id, scroll_id) = setup_scroll_read_scene();
        engine
            .get_entity_mut(npc_id)
            .expect("scroll owner exists")
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(200.0, 100.0));
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchScrollRead {
                actor: pc_id,
                target: npc_id,
                running: true,
            },
        );

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
        let running_titbit = engine
            .players
            .macro_store
            .get(pc_id)
            .and_then(|state| state.get_slot_titbit(0))
            .expect("running scroll read stores its QA titbit");
        assert!(
            engine
                .feedback
                .titbit_manager
                .is_running_for_qa(running_titbit)
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );

        assert_eq!(
            engine.orders.sequence_manager.sequence_count(),
            1,
            "quick-action startup clones the recorded scroll route once"
        );
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("running scroll read launches its stored route");
        let seek = sequence.get(0).expect("scroll route begins with Seek");
        assert_eq!(seek.command, Command::Seek);
        let SequenceElementData::Movement {
            action,
            element,
            post_seek_sequence,
            ..
        } = &seek.data
        else {
            panic!("running scroll route must begin with movement");
        };
        assert_eq!(*action, crate::order::OrderType::RunningUpright);
        assert_eq!(*element, Some(npc_id));
        assert_scroll_read_composite(
            post_seek_sequence
                .as_ref()
                .expect("recorded scroll route retains its continuation"),
            pc_id,
            npc_id,
            scroll_id,
        );
    }

    #[test]
    fn waking_up_validity_uses_sprite_action_distance() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Resuscitate, 0)]);
        {
            let pc = engine.get_entity_mut(pc_id).unwrap().element_data_mut();
            pc.set_position_map(crate::coordinates::MapPoint { x: 100.0, y: 100.0 });
            pc.set_direction_instantly(0);
        }
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::WakingUp,
            crate::coordinates::SpriteLocalPoint::new(33.0, 0.0),
            crate::coordinates::SpriteAnchor::new(10.0, 0.0),
        );
        let mut victim = ActorPc {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Lying);
                initial_element.kind = ElementKind::ActorPc;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData {
                unconscious: true,
                ..HumanData::default()
            },
            pc: PcData::default(),
        };
        victim
            .element
            .set_position_map(crate::coordinates::MapPoint { x: 143.0, y: 100.0 });
        let victim_id = engine.add_entity(Entity::Pc(victim));
        let element =
            SequenceElement::new_interaction(1, Command::WakeUp, Some(pc_id), Some(victim_id));

        assert!(engine.check_sequence_element_validity(&assets, pc_id, &element, true));

        engine
            .get_entity_mut(victim_id)
            .unwrap()
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint { x: 144.0, y: 100.0 });
        assert!(!engine.check_sequence_element_validity(&assets, pc_id, &element, true));
    }

    #[test]
    fn custom_pc_can_wake_only_same_allegiance_pc() {
        let (mut engine, mut assets, pc_id) = setup_pc_engine(&[]);
        std::sync::Arc::make_mut(&mut assets.profile_manager).characters[0].contextual_actions[0] =
            Action::Resuscitate;
        engine
            .get_entity_mut(pc_id)
            .unwrap()
            .pc_data_mut()
            .unwrap()
            .cached_camp = Camp::Custom(2);

        let add_unconscious_pc = |engine: &mut EngineInner, camp| {
            engine.add_entity(Entity::Pc(ActorPc {
                element: {
                    let mut initial_element = ElementData::from_initial_posture(Posture::Lying);
                    initial_element.kind = ElementKind::ActorPc;
                    initial_element.active = true;
                    initial_element
                },
                actor: ActorData::default(),
                human: HumanData {
                    unconscious: true,
                    ..HumanData::default()
                },
                pc: PcData {
                    cached_camp: camp,
                    life_points: 50,
                    ..PcData::default()
                },
            }))
        };
        let ally = add_unconscious_pc(&mut engine, Camp::Custom(2));
        let enemy = add_unconscious_pc(&mut engine, Camp::Custom(3));

        assert_eq!(
            determine_use_command(&engine, &assets, pc_id, ally),
            Some(Command::WakeUp)
        );
        assert_eq!(determine_use_command(&engine, &assets, pc_id, enemy), None);
        assert_eq!(
            engine.choose_use_cursor(&assets, ally, Some(pc_id)),
            crate::resource_ids::RHMOUSE_WAKE_UP
        );
        assert_eq!(
            engine.choose_use_cursor(&assets, enemy, Some(pc_id)),
            crate::resource_ids::RHMOUSE_DEFAULT
        );
    }

    #[test]
    fn tied_npc_use_prioritizes_loot_then_untie_and_setting_restores_original_behavior() {
        let (mut engine, mut assets, pc_id) = setup_pc_engine(&[]);
        std::sync::Arc::make_mut(&mut assets.profile_manager).characters[0].contextual_actions[..2]
            .copy_from_slice(&[Action::Tie, Action::Search]);
        let target_id = engine.add_entity(Entity::Soldier(ActorSoldier {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Tied);
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            // Script-created tied actors need not have been knocked out.
            human: HumanData {
                unconscious: false,
                ..HumanData::default()
            },
            npc: {
                NpcData {
                    ai: crate::element::AiActorData {
                        money: 12,
                        ..Default::default()
                    },
                    ..Default::default()
                }
            },
            soldier: SoldierData {
                cached_camp: Camp::Lacklandists,
                ..SoldierData::default()
            },
        }));

        assert_eq!(
            determine_use_command(&engine, &assets, pc_id, target_id),
            Some(Command::SearchCmd)
        );
        assert_eq!(
            engine.choose_use_cursor(&assets, target_id, Some(pc_id)),
            crate::resource_ids::RHMOUSE_SEARCH
        );
        let search =
            SequenceElement::new_interaction(1, Command::SearchCmd, Some(pc_id), Some(target_id));
        assert!(
            engine.check_sequence_element_validity(&assets, pc_id, &search, true),
            "a script-authored conscious tied NPC must remain searchable before release"
        );

        engine
            .get_entity_mut(target_id)
            .and_then(Entity::npc_data_mut)
            .expect("test target is an NPC")
            .money = 0;
        assert_eq!(
            determine_use_command(&engine, &assets, pc_id, target_id),
            Some(Command::Untie)
        );
        assert_eq!(
            engine.choose_use_cursor(&assets, target_id, Some(pc_id)),
            crate::resource_ids::RHMOUSE_TIE
        );
        let untie =
            SequenceElement::new_interaction(1, Command::Untie, Some(pc_id), Some(target_id));
        assert!(engine.check_sequence_element_validity(&assets, pc_id, &untie, true));

        std::sync::Arc::make_mut(&mut assets.profile_manager).characters[0].contextual_actions[0] =
            Action::NoAction;
        assert_eq!(
            determine_use_command(&engine, &assets, pc_id, target_id),
            None
        );
        assert_eq!(
            engine.choose_use_cursor(&assets, target_id, Some(pc_id)),
            crate::resource_ids::RHMOUSE_DEFAULT
        );
        assert!(!engine.check_sequence_element_validity(&assets, pc_id, &untie, false));
        std::sync::Arc::make_mut(&mut assets.profile_manager).characters[0].contextual_actions[0] =
            Action::Tie;

        engine
            .get_entity_mut(target_id)
            .expect("test target remains present")
            .set_posture(Posture::Lying);
        engine
            .get_entity_mut(pc_id)
            .and_then(Entity::actor_data_mut)
            .expect("test owner remains a PC")
            .active_ability = crate::movement::ActiveAbility {
            kind: Some(crate::movement::AbilityKind::Untie),
            done_effect_applied: true,
            strangle_initialized: false,
            sequence_id: Some(crate::sequence::SequenceId(9)),
            element_index: 0,
            target: Some(target_id),
            order_id: std::num::NonZeroU32::new(91),
        };
        assert!(
            engine.check_sequence_element_validity(&assets, pc_id, &untie, true),
            "remaining reversed frames must stay valid after DONE releases the target"
        );

        engine.control.sim_config.enable_unbinding = false;
        assert!(
            engine.check_sequence_element_validity(&assets, pc_id, &untie, true),
            "a setting edit must not cancel an already accepted release"
        );
        engine
            .get_entity_mut(pc_id)
            .and_then(Entity::actor_data_mut)
            .expect("test owner remains a PC")
            .active_ability
            .clear();
        engine
            .get_entity_mut(target_id)
            .expect("test target remains present")
            .set_posture(Posture::Tied);
        assert_eq!(
            determine_use_command(&engine, &assets, pc_id, target_id),
            None
        );
        assert_eq!(
            engine.choose_use_cursor(&assets, target_id, Some(pc_id)),
            crate::resource_ids::RHMOUSE_DEFAULT
        );
        assert!(!engine.check_sequence_element_validity(&assets, pc_id, &untie, false));
        assert!(!engine.check_sequence_element_validity(&assets, pc_id, &search, false));
    }

    #[test]
    fn untie_quick_action_uses_tie_phase() {
        assert_eq!(
            recorded_interaction_quick_phase(Command::Untie),
            Some(QuickAction::Tie)
        );
    }

    #[test]
    fn drop_ale_seek_tolerance_uses_sprite_action_distance() {
        let (mut engine, _assets, pc_id) = setup_pc_engine(&[(Action::Ale, 1)]);
        {
            let pc = engine.get_entity_mut(pc_id).unwrap().element_data_mut();
            pc.set_position_map(crate::coordinates::MapPoint { x: 20.0, y: 30.0 });
            pc.set_direction_instantly(0);
        }
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::DroppingAle,
            crate::coordinates::SpriteLocalPoint::new(13.0, 0.0),
            crate::coordinates::SpriteAnchor::new(0.0, 0.0),
        );

        engine.apply_drop_ale_at(
            pc_id,
            crate::coordinates::MapPoint { x: 80.0, y: 90.0 },
            false,
            false,
            None,
            None,
            None,
        );

        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .unwrap();
        let seek = sequence.get(0).unwrap();
        match &seek.data {
            SequenceElementData::Movement { tolerance, .. } => {
                assert!((*tolerance - 13.0).abs() < 0.001);
            }
            other => panic!("expected movement seek element, got {other:?}"),
        }
    }

    #[test]
    fn mapped_interaction_seek_tolerance_uses_uword_sprite_action_distance() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let (mut engine, _assets, pc_id) = setup_pc_engine(&[(Action::Search, 0)]);
        {
            let pc = engine.get_entity_mut(pc_id).unwrap().element_data_mut();
            pc.set_position_map(crate::coordinates::MapPoint { x: 10.0, y: 10.0 });
            pc.set_direction_instantly(0);
        }
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::Searching,
            crate::coordinates::SpriteLocalPoint::new(19.75, 0.0),
            crate::coordinates::SpriteAnchor::new(0.0, 0.0),
        );
        let target_id = spawn_pc_at(&mut engine, 90.0, 10.0);

        engine.apply_interaction_with_seek(sim, pc_id, target_id, Command::SearchCmd, false);

        assert_eq!(first_seek_tolerance(&engine), 19.0);
    }

    #[test]
    fn pc_in_coma_carry_keeps_fractional_action_distance_plus_ten() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, _assets, pc_id) = setup_pc_engine(&[]);
        {
            let pc = engine.get_entity_mut(pc_id).unwrap().element_data_mut();
            pc.set_position_map(crate::coordinates::MapPoint { x: 10.0, y: 10.0 });
            pc.set_direction_instantly(0);
        }
        let lift_distance = f32::from_bits(0x41a1_dcb0); // 20.232757
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::TransitionWaitingUprightCarryingCorpse,
            crate::coordinates::SpriteLocalPoint::new(lift_distance, 0.0),
            crate::coordinates::SpriteAnchor::new(0.0, 0.0),
        );
        let target_id = spawn_pc_at(&mut engine, 90.0, 10.0);
        {
            let target = engine.get_entity_mut(target_id).unwrap();
            target
                .element_data_mut()
                .publish_order_posture(Posture::Lying);
            target.human_data_mut().unwrap().unconscious = true;
        }
        let target_description_index = engine
            .get_entity(target_id)
            .and_then(Entity::pc_data)
            .and_then(|pc| pc.campaign_description_index)
            .expect("test PC has a campaign description")
            as usize;
        engine.mission_domain.campaign.characters[target_description_index]
            .status
            .in_coma = true;

        engine.apply_interaction_with_seek(&sim, pc_id, target_id, Command::TakeCorpse, false);

        assert_eq!(first_seek_tolerance(&engine).to_bits(), 0x41f1_dcb0);
        let seek = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .unwrap()
            .get(0)
            .unwrap();
        let SequenceElementData::Movement { flags, .. } = &seek.data else {
            panic!("PC in-coma carry must start with Seek");
        };
        assert!(flags.contains(MoveFlags::SEEK));
        assert!(
            !flags.contains(MoveFlags::SEEK_IN_BUILDINGS),
            "PC-specific carrying does not pass the human click handler's building flag"
        );
    }

    #[test]
    fn unconscious_pc_outside_coma_uses_human_take_corpse_distance() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, _assets, pc_id) = setup_pc_engine(&[]);
        {
            let pc = engine.get_entity_mut(pc_id).unwrap().element_data_mut();
            pc.set_position_map(crate::coordinates::MapPoint { x: 10.0, y: 10.0 });
            pc.set_direction_instantly(0);
        }
        let lift_distance = f32::from_bits(0x4116_2058); // 9.382896
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::TransitionWaitingUprightCarryingCorpse,
            crate::coordinates::SpriteLocalPoint::new(lift_distance, 0.0),
            crate::coordinates::SpriteAnchor::new(0.0, 0.0),
        );
        let target_id = spawn_pc_at(&mut engine, 90.0, 10.0);
        {
            let target = engine.get_entity_mut(target_id).unwrap();
            target
                .element_data_mut()
                .publish_order_posture(Posture::Lying);
            target.human_data_mut().unwrap().unconscious = true;
        }

        engine.apply_interaction_with_seek(&sim, pc_id, target_id, Command::TakeCorpse, false);

        assert_eq!(first_seek_tolerance(&engine), 9.0);
    }

    #[test]
    fn fx_target_click_commands_use_zero_tolerance_move_and_preserve_wait_time() {
        let commands = [
            Command::SearchCmd,
            Command::UseLever,
            Command::HitTarget,
            Command::HandleTarget,
            Command::TakeTarget,
            Command::Pay,
        ];

        for command in commands {
            let sim = crate::sim_rng::test_context();
            let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
            engine.scripts.mission = Some(minimal_script());
            let sector = crate::position_interface::SectorHandle::new(1);
            {
                let pc = engine.get_entity_mut(pc_id).expect("test PC exists");
                pc.element_data_mut()
                    .set_position_map(crate::coordinates::MapPoint::new(100.0, 100.0));
                pc.element_data_mut().set_sector(sector);
                pc.element_data_mut().sprite.position_iface.set_move_box(
                    crate::coordinates::MoveBox::from_coords(-6.0, -4.0, 6.0, 4.0),
                );
                pc.actor_data_mut()
                    .expect("test PC has actor data")
                    .wait_time = 0xffff_ff3e;
            }

            let mut target = ElementTarget {
                element: {
                    let mut initial_element = ElementData::default();
                    initial_element.kind = ElementKind::Target;
                    initial_element.active = true;
                    initial_element
                },
                fx: FxData::default(),
                target: TargetData::default(),
            };
            target
                .element
                .set_position_map(crate::coordinates::MapPoint::new(300.0, 100.0));
            target.element.set_sector(sector);
            let target_id = engine.add_entity(Entity::Target(target));
            bind_single_action_point(
                &mut engine,
                target_id,
                crate::order::OrderType::WaitingUpright,
                crate::coordinates::SpriteLocalPoint::ZERO,
                crate::coordinates::SpriteAnchor::ZERO,
            );
            engine
                .get_entity_mut(target_id)
                .expect("target exists after sprite binding")
                .element_data_mut()
                .set_sector(sector);

            let mut display = HostDisplayState::default();
            let mut input = InputState::default();
            engine.apply_command(
                &sim,
                &mut display,
                &mut input,
                &assets,
                &PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target: target_id,
                    command,
                    running: false,
                },
            );

            let route = engine
                .orders
                .sequence_manager
                .sequences_iter()
                .next()
                .expect("target click launches its direct route");
            let movement = route.get(0).expect("target route starts with movement");
            assert_eq!(movement.command, Command::Move, "command {command:?}");
            let SequenceElementData::Movement {
                element,
                tolerance,
                flags,
                ..
            } = &movement.data
            else {
                panic!("target route must start with movement for {command:?}");
            };
            assert_eq!(*element, Some(target_id), "command {command:?}");
            assert_eq!(*tolerance, 0.0, "command {command:?}");
            assert!(!flags.contains(MoveFlags::SEEK), "command {command:?}");

            engine.hourglass_phase_sequences(&sim, &mut HostDisplayState::default(), &assets);
            assert_eq!(
                engine
                    .get_entity(pc_id)
                    .and_then(Entity::actor_data)
                    .expect("test PC retains actor data")
                    .wait_time,
                0xffff_ff3e,
                "ordinary target movement must not arm seek refresh for {command:?}"
            );
        }
    }

    #[test]
    fn recorded_fx_target_replays_authored_coordinate_seek_and_continuation() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let sector = crate::position_interface::SectorHandle::new(3);
        {
            let pc = engine.get_entity_mut(pc_id).expect("test PC exists");
            pc.element_data_mut()
                .set_position_map(crate::coordinates::MapPoint::new(100.0, 100.0));
            pc.element_data_mut().set_sector(sector);
            pc.element_data_mut()
                .publish_order_posture(Posture::Crouched);
        }

        let mut target = ElementTarget {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::Target;
                initial_element.active = true;
                initial_element
            },
            fx: FxData::default(),
            target: TargetData::default(),
        };
        let recorded_destination = crate::coordinates::MapPoint::new(300.0, 120.0);
        target.element.set_position_map(recorded_destination);
        target.element.set_sector(sector);
        target.element.set_layer(4);
        let target_id = engine.add_entity(Entity::Target(target));
        bind_single_action_point(
            &mut engine,
            target_id,
            crate::order::OrderType::WaitingUpright,
            crate::coordinates::SpriteLocalPoint::new(11.0, 7.0),
            crate::coordinates::SpriteAnchor::ZERO,
        );
        {
            let target = engine
                .get_entity_mut(target_id)
                .expect("target exists after sprite binding")
                .element_data_mut();
            target.set_sector(sector);
            target.set_layer(4);
        }
        let recorded_turn_point = engine
            .get_entity(target_id)
            .and_then(Entity::current_gameplay_point_map)
            .expect("bound target has a current point");

        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target: target_id,
                command: Command::HitTarget,
                running: false,
            },
        );

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
        let state = engine
            .players
            .macro_store
            .get(pc_id)
            .expect("target interaction was recorded");
        let recorded = state.slot(0).expect("slot zero exists");
        assert_eq!(recorded.steps.len(), 1);
        assert_eq!(
            recorded.steps[0].replay,
            QaReplayCommand::TargetInteraction {
                target: target_id,
                command: Command::HitTarget,
                destination: recorded_destination,
                sector,
                layer: 4,
                action: crate::order::OrderType::WalkingCrouched,
                turn_point: recorded_turn_point,
            }
        );

        // Playback clones the recorded sequence. Moving the target after
        // recording must not rewrite the coordinate seek or turn geometry.
        engine
            .get_entity_mut(target_id)
            .expect("target still exists")
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(700.0, 500.0));
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );

        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("recorded target route launches one seek");
        assert_eq!(sequence.len(), 1);
        let seek = sequence.get(0).expect("recorded route starts with seek");
        assert_eq!(seek.command, Command::Seek);
        let SequenceElementData::Movement {
            destination,
            sector: seek_sector,
            layer,
            element,
            tolerance,
            flags,
            action,
            post_seek_sequence,
            ..
        } = &seek.data
        else {
            panic!("recorded target route must start with coordinate movement");
        };
        assert_eq!(*destination, recorded_destination);
        assert_eq!(*seek_sector, sector);
        assert_eq!(*layer, 4);
        assert_eq!(*element, None);
        assert_eq!(*tolerance, 0.0);
        assert_eq!(*flags, MoveFlags::empty());
        assert_eq!(*action, crate::order::OrderType::WalkingCrouched);

        let post_seek = post_seek_sequence
            .as_ref()
            .expect("recorded seek retains Turn and interaction");
        assert_eq!(post_seek.len(), 2);
        let turn = post_seek.get(0).expect("Turn follows seek");
        assert_eq!(turn.command, Command::Turn);
        assert_eq!(turn.command_level, 1);
        assert!(matches!(
            turn.get_property(Field::CameraPoint),
            Some(FieldValue::GeoPoint2D { x, y })
                if *x == recorded_turn_point.x && *y == recorded_turn_point.y
        ));
        let interaction = post_seek.get(1).expect("interaction follows Turn");
        assert_eq!(interaction.command, Command::HitTarget);
        assert_eq!(interaction.command_level, 2);
        assert!(matches!(
            interaction.data,
            SequenceElementData::Interaction {
                antagonist: Some(id)
            } if id == target_id
        ));
    }

    #[test]
    fn same_command_against_human_keeps_generic_entity_seek() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let sector = crate::position_interface::SectorHandle::new(1);
        {
            let pc = engine.get_entity_mut(pc_id).expect("test PC exists");
            pc.element_data_mut()
                .set_position_map(crate::coordinates::MapPoint::new(100.0, 100.0));
            pc.element_data_mut().set_sector(sector);
            pc.actor_data_mut()
                .expect("test PC has actor data")
                .wait_time = 7;
        }
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::Searching,
            crate::coordinates::SpriteLocalPoint::new(13.0, 0.0),
            crate::coordinates::SpriteAnchor::ZERO,
        );
        engine
            .get_entity_mut(pc_id)
            .expect("test PC exists after sprite binding")
            .element_data_mut()
            .set_sector(sector);
        engine
            .get_entity_mut(pc_id)
            .expect("test PC exists after sprite binding")
            .element_data_mut()
            .sprite
            .position_iface
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -6.0, -4.0, 6.0, 4.0,
            ));
        let target_id = spawn_pc_at(&mut engine, 300.0, 100.0);
        engine
            .get_entity_mut(target_id)
            .expect("target PC exists")
            .element_data_mut()
            .set_sector(sector);

        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target: target_id,
                command: Command::SearchCmd,
                running: false,
            },
        );

        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("generic human interaction launches a seek");
        let seek = sequence.get(0).expect("seek is the first element");
        assert_eq!(seek.command, Command::Seek);
        let SequenceElementData::Movement {
            tolerance, flags, ..
        } = &seek.data
        else {
            panic!("generic interaction starts with movement");
        };
        assert_eq!(*tolerance, 13.0);
        assert!(flags.contains(MoveFlags::SEEK));

        engine.hourglass_phase_sequences(&sim, &mut HostDisplayState::default(), &assets);
        assert_eq!(
            engine
                .get_entity(pc_id)
                .and_then(Entity::actor_data)
                .expect("test PC retains actor data")
                .wait_time,
            25
        );
    }

    #[test]
    fn pay_seek_faces_the_beggar_action_point() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let (mut engine, _assets, pc_id) = setup_pc_engine(&[]);
        {
            let pc = engine.get_entity_mut(pc_id).unwrap().element_data_mut();
            pc.set_position_map(crate::coordinates::MapPoint { x: 10.0, y: 10.0 });
            pc.set_direction_instantly(0);
        }
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::Paying,
            crate::coordinates::SpriteLocalPoint::new(8.0, 6.0),
            crate::coordinates::SpriteAnchor::ZERO,
        );
        // The original game's pay command is created only by
        // civilian clicking after the beggar check succeeds;
        // its unconditional post-click cooldown stamp therefore targets that
        // same civilian. Keep this direct helper test within that contract.
        let target_id = spawn_friendly_civilian(&mut engine);
        engine
            .get_entity_mut(target_id)
            .expect("beggar exists")
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint { x: 90.0, y: 10.0 });

        engine.apply_interaction_with_seek(sim, pc_id, target_id, Command::Pay, false);

        assert_eq!(friendly_beggar_dont_talk_counter(&engine, target_id), 3);

        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .expect("Pay registers its seek sequence");
        let seek = sequence.get(0).expect("Pay seek is first");
        match &seek.data {
            SequenceElementData::Movement {
                flags, tolerance, ..
            } => {
                assert!(flags.contains(MoveFlags::SEEK));
                assert!(flags.contains(MoveFlags::USE_POINT));
                assert_eq!(
                    *tolerance, 0.0,
                    "Original Pay passes literal action distance zero instead of the Paying sprite hotspot distance"
                );
            }
            other => panic!("expected Pay movement seek element, got {other:?}"),
        }
    }

    #[test]
    fn running_non_recording_pay_stamps_beggar_and_only_makes_current_order_fast() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, _assets, pc_id) = setup_pc_engine(&[]);
        let target_id = spawn_friendly_civilian(&mut engine);

        let movement = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(pc_id),
            crate::order::OrderType::WalkingUpright,
        );
        let movement_sequence = engine.orders.sequence_manager.launch_element(movement);
        engine
            .orders
            .sequence_manager
            .element_in_progress(movement_sequence, 0);

        engine.apply_interaction_with_seek(&sim, pc_id, target_id, Command::Pay, true);

        assert_eq!(friendly_beggar_dont_talk_counter(&engine, target_id), 3);
        assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);
        let current = engine
            .orders
            .sequence_manager
            .get_element(movement_sequence, 0)
            .expect("the preexisting movement remains selected");
        let SequenceElementData::Movement { action, flags, .. } = &current.data else {
            panic!("preexisting movement changed kind");
        };
        assert_eq!(*action, crate::order::OrderType::RunningUpright);
        assert!(flags.contains(MoveFlags::FAST));
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .all(|element| !matches!(element.command, Command::Pay | Command::Seek))
        );
    }

    #[test]
    fn recorded_beggar_click_stamp_restores_discarded_double_click_side_effect() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, _pc_id) = setup_pc_engine(&[]);
        let beggar_id = spawn_friendly_civilian(&mut engine);

        engine.apply_command(
            &sim,
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::BeggarDontTalkStamp { beggar_id },
        );

        assert_eq!(friendly_beggar_dont_talk_counter(&engine, beggar_id), 3);
    }

    #[test]
    fn running_non_pay_does_not_stamp_friendly_target() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, _assets, pc_id) = setup_pc_engine(&[]);
        let target_id = spawn_friendly_civilian(&mut engine);
        let Some(Entity::Civilian(civilian)) = engine.get_entity_mut(target_id) else {
            unreachable!("new friendly target changed kind");
        };
        let crate::element::AiBrain::Friendly(ai) = &mut civilian.npc.ai_brain else {
            unreachable!("new friendly target changed AI kind");
        };
        ai.set_beggar_dont_talk_counter(2);

        engine.apply_interaction_with_seek(&sim, pc_id, target_id, Command::SearchCmd, true);

        assert_eq!(friendly_beggar_dont_talk_counter(&engine, target_id), 2);
        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
    }

    #[test]
    fn swordstrike_down_uses_original_literal_seek_distance() {
        assert_eq!(interaction_distance(Command::SwordstrikeDown), 40.0);
    }

    #[test]
    fn shoot_bow_interaction_launches_without_seek() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let (mut engine, _assets, pc_id) = setup_pc_engine(&[(Action::Bow, 1)]);
        {
            let pc = engine.get_entity_mut(pc_id).unwrap().element_data_mut();
            pc.set_position_map(crate::coordinates::MapPoint { x: 10.0, y: 10.0 });
        }
        let target_id = spawn_pc_at(&mut engine, 90.0, 10.0);

        engine.apply_interaction_with_seek(sim, pc_id, target_id, Command::ShootBow, false);

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);
        let sequence = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .next()
            .unwrap();
        let element = sequence.get(0).unwrap();
        assert_eq!(element.command, Command::ShootBow);
        assert!(matches!(
            element.data,
            SequenceElementData::Interaction { .. }
        ));
    }

    #[test]
    fn mapped_interaction_missing_sprite_action_distance_noops() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let (mut engine, _assets, pc_id) = setup_pc_engine(&[(Action::Hit, 0)]);
        let target_id = spawn_pc_at(&mut engine, 90.0, 10.0);

        engine.apply_interaction_with_seek(sim, pc_id, target_id, Command::HitCmd, false);

        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .next()
                .is_none()
        );
    }

    #[test]
    fn climb_on_shoulders_seek_tolerance_matches_original_literal() {
        let (mut engine, _assets, pc_id) = setup_pc_engine(&[(Action::Climb, 0)]);
        {
            let pc = engine.get_entity_mut(pc_id).unwrap().element_data_mut();
            pc.set_position_map(crate::coordinates::MapPoint { x: 10.0, y: 10.0 });
            pc.set_direction_instantly(0);
        }
        bind_single_action_point(
            &mut engine,
            pc_id,
            crate::order::OrderType::ClimbingUpOnShoulders,
            crate::coordinates::SpriteLocalPoint::new(11.0, 0.0),
            crate::coordinates::SpriteAnchor::new(0.0, 0.0),
        );
        let target_id = spawn_pc_at(&mut engine, 90.0, 10.0);

        engine.apply_climb_on_shoulders_with_seek(pc_id, target_id, false);

        assert!((first_seek_tolerance(&engine) - 8.0).abs() < 0.001);
    }

    #[test]
    fn pickup_dispatch_landed_net_returns_take() {
        // Landed nets always route to Seek+Take regardless of
        // takability.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Net, 1)]);
        let id = spawn_net(&mut engine, false);
        assert_eq!(
            object_pickup_command(&engine, &assets, id, pc_id),
            Some(Command::Take)
        );
    }

    #[test]
    fn pickup_dispatch_flying_net_returns_none() {
        // A net still in the air isn't pickable until it lands.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Net, 1)]);
        let id = spawn_net(&mut engine, true);
        assert_eq!(object_pickup_command(&engine, &assets, id, pc_id), None);
    }

    #[test]
    fn pickup_dispatch_bonus_returns_take_when_storage_free() {
        // PC has Heal action + storage slot open → take.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Heal, 3)]);
        let id = spawn_bonus(&mut engine, ObjectType::BonusPlants, true, Action::Heal);
        assert_eq!(
            object_pickup_command(&engine, &assets, id, pc_id),
            Some(Command::Take)
        );
    }

    #[test]
    fn pickup_dispatch_resolves_exact_campaign_description_identity() {
        // Original-game PCs read ammo from their own status, not from a
        // campaign array slot equal to the first matching profile or
        // the list index.
        let (mut engine, assets, pc_id) =
            setup_pc_engine_with_split_profile_and_status(&[(Action::Bow, 12)]);
        let id = spawn_bonus(&mut engine, ObjectType::BonusArrow, true, Action::Bow);
        assert_eq!(
            object_pickup_command(&engine, &assets, id, pc_id),
            Some(Command::Take)
        );
    }

    #[test]
    fn pc_action_disable_uses_profile_slot_not_action_enum_value() {
        // Bow's enum value is 1, but this profile places it in
        // portrait slot 0. The original game's action lookup disables the
        // portrait slot.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Bow, 12)]);
        if let Some(pc) = engine.get_entity_mut(pc_id).and_then(|e| e.pc_data_mut()) {
            pc.disabled_actions = vec![false, false, false];
            pc.current_action = Action::Bow;
            pc.saved_action = Action::Bow;
        }

        engine.disable_pc_action(&assets, pc_id, Action::Bow);

        let pc = engine
            .get_entity(pc_id)
            .and_then(|e| e.pc_data())
            .expect("test PC exists");
        assert_eq!(pc.disabled_actions, [true, false, false]);
        assert_eq!(pc.current_action, Action::NoAction);
        assert_eq!(pc.saved_action, Action::NoAction);
    }

    #[test]
    fn pc_action_enable_uses_profile_slot_not_action_enum_value() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Bow, 12)]);
        if let Some(pc) = engine.get_entity_mut(pc_id).and_then(|e| e.pc_data_mut()) {
            pc.disabled_actions = vec![true, false, false];
        }

        engine.enable_pc_action(&assets, pc_id, Action::Bow);

        let pc = engine
            .get_entity(pc_id)
            .and_then(|e| e.pc_data())
            .expect("test PC exists");
        assert_eq!(pc.disabled_actions, [false, false, false]);
    }

    #[test]
    fn pickup_dispatch_bonus_returns_none_when_storage_full() {
        // PC has the action but current ammo == max → reject.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Heal, 3)]);
        if let Some(campaign) = Some(&mut engine.mission_domain.campaign)
            && let Some(pc_desc) = campaign.characters.get_mut(0)
        {
            pc_desc.status.set_ammo(Action::Heal, 3);
        }
        let id = spawn_bonus(&mut engine, ObjectType::BonusPlants, true, Action::Heal);
        assert_eq!(object_pickup_command(&engine, &assets, id, pc_id), None);
    }

    #[test]
    fn pickup_dispatch_preserves_over_capacity_unsigned_storage_left() {
        // A save can retain Normal-mode ammo after switching to Hard, where
        // the profile maximum drops from 6 to 4. Original's signed
        // subtraction is assigned to an unsigned 32-bit value, so this remains takable.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Apple, 6)]);
        engine.control.sim_config.difficulty = crate::player_profile::DifficultyLevel::Hard;
        engine.mission_domain.campaign.characters[0]
            .status
            .set_ammo(Action::Apple, 6);
        let id = spawn_bonus(&mut engine, ObjectType::BonusApple, true, Action::Apple);

        assert_eq!(
            object_pickup_command(&engine, &assets, id, pc_id),
            Some(Command::Take)
        );
    }

    #[test]
    fn pickup_dispatch_bonus_returns_none_when_pc_lacks_action() {
        // PC profile lacks the bonus's associated_action → not
        // takable; click silently ignored.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Bow, 12)]);
        let id = spawn_bonus(&mut engine, ObjectType::BonusPlants, true, Action::Heal);
        assert_eq!(object_pickup_command(&engine, &assets, id, pc_id), None);
    }

    #[test]
    fn pickup_dispatch_eat_bonus_routes_through_guzzle() {
        // PC lacks Eat but has Guzzle with storage left → still takable.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Guzzle, 2)]);
        let id = spawn_bonus(&mut engine, ObjectType::BonusLambLeg, true, Action::Eat);
        assert_eq!(
            object_pickup_command(&engine, &assets, id, pc_id),
            Some(Command::Take)
        );
    }

    #[test]
    fn pickup_dispatch_taken_bonus_returns_none() {
        // `is_takable` flips off once `taken` is set.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Heal, 3)]);
        let id = spawn_bonus(&mut engine, ObjectType::BonusPlants, true, Action::Heal);
        if let Some(Entity::Bonus(b)) = engine.get_entity_mut(id) {
            b.object.taken = true;
        }
        assert_eq!(object_pickup_command(&engine, &assets, id, pc_id), None);
    }

    #[test]
    fn pickup_dispatch_relic_bonus_uses_explicit_take() {
        // Original-game bonus takeability delegates relics to the
        // base NoAction-object path, which queues Seek -> Take.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let id = spawn_bonus(
            &mut engine,
            ObjectType::BonusAmpulla,
            true,
            Action::NoAction,
        );
        assert_eq!(
            object_pickup_command(&engine, &assets, id, pc_id),
            Some(Command::Take)
        );
    }

    #[test]
    fn pickup_dispatch_invisible_scroll_returns_none() {
        // Only Visible / Opened scrolls are focusable — Invisible
        // scrolls are pre-reveal and aren't clickable until the
        // beggar reveal flow runs.  (Visible/Opened → Take is covered
        // by `determine_use_command`; exercising it from a unit test
        // would require a fully-initialised `MissionScript`.)
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let id = spawn_scroll(&mut engine, true);
        assert_eq!(engine.scroll_status(id), ScrollStatus::Invisible);
        assert_eq!(object_pickup_command(&engine, &assets, id, pc_id), None);
    }

    #[test]
    fn pickup_dispatch_landed_coin_returns_take() {
        // Coin on the ground: falls through to the base Seek+Take
        // once the source purse has already been taken (or was never
        // set).  Coins have `associated_action = NoAction` so
        // takability is vacuously true.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let id = spawn_projectile(&mut engine, ObjectType::Coin, false, Action::NoAction);
        assert_eq!(
            object_pickup_command(&engine, &assets, id, pc_id),
            Some(Command::Take)
        );
    }

    #[test]
    fn pickup_dispatch_flying_coin_returns_none() {
        // In-flight coins (just ejected from a burst purse) aren't
        // clickable until they land.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let id = spawn_projectile(&mut engine, ObjectType::Coin, true, Action::NoAction);
        assert_eq!(object_pickup_command(&engine, &assets, id, pc_id), None);
    }

    #[test]
    fn pickup_dispatch_landed_apple_returns_none() {
        // Apples are throwable bait, not pickups, so the dispatch
        // rejects them defensively.
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Apple, 3)]);
        let id = spawn_projectile(&mut engine, ObjectType::Apple, false, Action::Apple);
        assert_eq!(object_pickup_command(&engine, &assets, id, pc_id), None);
    }

    #[test]
    fn coin_click_forwards_to_live_source_purse() {
        // When the source purse is still on the ground (not taken),
        // the click is forwarded to the purse so the take handler
        // collects every sibling coin in one sweep.
        let (mut engine, _assets, _pc_id) = setup_pc_engine(&[]);
        let purse_id = engine.add_entity(Entity::Projectile(ElementProjectile {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ObjectProjectile;
                initial_element.active = true;
                initial_element
            },
            object: ObjectData {
                object_type: ObjectType::Purse,
                ..Default::default()
            },
            projectile: ProjectileData::default(),
        }));
        let coin_id = engine.add_entity(Entity::Projectile(ElementProjectile {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ObjectProjectile;
                initial_element.active = true;
                initial_element
            },
            object: ObjectData {
                object_type: ObjectType::Coin,
                ..Default::default()
            },
            projectile: ProjectileData {
                purse: crate::element::PurseData {
                    source_purse: Some(purse_id),
                    ..crate::element::PurseData::default()
                },
                ..Default::default()
            },
        }));
        assert_eq!(coin_pickup_target(&engine, coin_id), purse_id);
    }

    #[test]
    fn coin_click_passes_through_when_purse_taken() {
        // If the source purse is `taken`, the forwarding branch is
        // skipped and the coin is taken individually.
        let (mut engine, _assets, _pc_id) = setup_pc_engine(&[]);
        let purse_id = engine.add_entity(Entity::Projectile(ElementProjectile {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ObjectProjectile;
                initial_element.active = true;
                initial_element
            },
            object: ObjectData {
                object_type: ObjectType::Purse,
                taken: true,
                ..Default::default()
            },
            projectile: ProjectileData::default(),
        }));
        let coin_id = engine.add_entity(Entity::Projectile(ElementProjectile {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ObjectProjectile;
                initial_element.active = true;
                initial_element
            },
            object: ObjectData {
                object_type: ObjectType::Coin,
                ..Default::default()
            },
            projectile: ProjectileData {
                purse: crate::element::PurseData {
                    source_purse: Some(purse_id),
                    ..crate::element::PurseData::default()
                },
                ..Default::default()
            },
        }));
        assert_eq!(coin_pickup_target(&engine, coin_id), coin_id);
    }

    #[test]
    fn coin_click_passes_through_when_loose() {
        // Loose coins (no `source_purse`) take individually.
        let (mut engine, _assets, _pc_id) = setup_pc_engine(&[]);
        let coin_id = spawn_projectile(
            &mut engine,
            ObjectType::Coin,
            false,
            crate::profiles::Action::NoAction,
        );
        assert_eq!(coin_pickup_target(&engine, coin_id), coin_id);
    }

    #[test]
    fn pickup_dispatch_non_object_returns_none() {
        // Civilians, soldiers, PCs etc. must not accidentally route
        // through the object pickup path — they have their own focus
        // handling (Interact / Sword / Use-beggar).
        let (engine, assets, pc_id) = setup_pc_engine(&[]);
        assert_eq!(
            object_pickup_command(
                &engine,
                &assets,
                EntityId::Pc(crate::entity_id::PcId(u32::MAX)),
                pc_id
            ),
            None
        );
    }

    #[test]
    fn unauthorized_seat_lifecycle_is_rejected_before_seat_allocation() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, _) = setup_pc_engine(&[]);
        let before = crate::replay::state_hash(&engine);
        let seat_count = engine.players.seats.len();

        engine.apply_frame_commands_with_mode(
            &sim,
            &assets,
            &[PlayerInput::new(
                PlayerId(12),
                PlayerCommand::ConnectSeat {
                    player_id: PlayerId(13),
                    nickname: "unauthorized".into(),
                },
            )],
            SelectionCommandBatchMode::InferNestedSelection,
        );

        assert_eq!(engine.players.seats.len(), seat_count);
        assert_eq!(crate::replay::state_hash(&engine), before);
    }

    #[test]
    fn unreachable_take_preflight_preserves_recording_and_simulation_state() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let target = spawn_scroll(&mut engine, true);
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();
        engine.apply_command(
            &sim,
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        assert!(engine.is_qa_recording_for(pc_id));
        assert!(!engine.object_take_reachable(pc_id, target));
        let before = crate::replay::state_hash(&engine);

        engine.apply_command_for_seat_with_replay_context(
            &sim,
            &mut CameraDisplayState::default(),
            &assets,
            0,
            &PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target,
                command: Command::Take,
                running: false,
            },
            false,
        );

        assert!(engine.is_qa_recording_for(pc_id));
        assert_eq!(crate::replay::state_hash(&engine), before);
    }

    #[test]
    #[should_panic(expected = "recorded interaction target")]
    fn invalid_recording_identity_reports_explicit_preflight_failure() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, pc_id, _) = setup_strangle_command_scene();
        engine.apply_command(
            &sim,
            &mut HostDisplayState::default(),
            &mut InputState::default(),
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        // Use the test harness's panic expectation: this repository's Cranelift
        // test backend does not reliably support catching and resuming unwinds.
        engine.apply_command_for_seat_with_replay_context(
            &sim,
            &mut CameraDisplayState::default(),
            &assets,
            0,
            &PlayerCommand::LaunchInteraction {
                actor: pc_id,
                target: EntityId::Soldier(crate::entity_id::SoldierId(u32::MAX)),
                command: Command::StrangleCmd,
                running: false,
            },
            false,
        );
    }

    #[test]
    fn connect_seat_creates_and_names_peer() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
        let (mut engine, assets, _pc_id) = setup_pc_engine(&[]);
        let mut input = InputState::default();
        let mut display = HostDisplayState::default();

        // Host issues a ConnectSeat for peer 2.  The dispatch `seat`
        // is HOST (0) but the command's payload targets PlayerId(2).
        engine.apply_commands(
            sim,
            &mut display,
            &mut input,
            &assets,
            &[PlayerInput::host(PlayerCommand::ConnectSeat {
                player_id: PlayerId(2),
                nickname: "alice".into(),
            })],
        );

        let seat2 = engine.seat(PlayerId(2)).expect("seat 2 must exist");
        assert!(seat2.connected);
        assert_eq!(seat2.nickname, "alice");
        // Seat 1 was lazy-grown to fill the gap but is inactive.
        let seat1 = engine.seat(PlayerId(1)).expect("seat 1 was filled");
        assert!(!seat1.is_active(1));
    }

    #[test]
    fn recorded_nested_cancel_is_the_only_select_pc_action_fanout() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::player_command::{PlayerCommand, PlayerInput};
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Bow, 10)]);
        let mut input = InputState::default();
        let mut display = HostDisplayState::default();
        engine
            .get_entity_mut(pc_id)
            .and_then(Entity::pc_data_mut)
            .expect("test PC data")
            .current_action = Action::Bow;

        engine.apply_commands(
            sim,
            &mut display,
            &mut input,
            &assets,
            &[
                PlayerInput::host(PlayerCommand::SelectPc {
                    pc_id,
                    append: false,
                }),
                PlayerInput::host(PlayerCommand::CancelAction { pc_id }),
            ],
        );

        assert_eq!(
            engine
                .get_entity(pc_id)
                .and_then(Entity::pc_data)
                .expect("test PC data")
                .current_action,
            Action::NoAction
        );
        assert!(
            !engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .any(|element| {
                    element.owner == Some(pc_id) && element.command == Command::EquipBow
                }),
            "the root SelectPc must not synthesize stale EquipBow before its recorded nested CancelAction"
        );
    }

    #[test]
    fn independent_adjacent_cancel_does_not_suppress_select_pc_action_fanout() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::player_command::{PlayerCommand, PlayerInput};
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Net, 1)]);
        let mut input = InputState::default();
        let mut display = HostDisplayState::default();
        engine
            .get_entity_mut(pc_id)
            .and_then(Entity::pc_data_mut)
            .expect("test PC data")
            .current_action = Action::Net;

        let mut wait = SequenceElement::new_generic(1, Command::WaitTimer, Some(pc_id));
        wait.priority = crate::sequence::SequencePriority::Wait;
        let wait_sequence = engine.orders.sequence_manager.launch_element(wait);
        engine
            .orders
            .sequence_manager
            .element_in_progress(wait_sequence, 0);

        engine.apply_commands_with_mode(
            sim,
            &mut display,
            &mut input,
            &assets,
            &[
                PlayerInput::host(PlayerCommand::SelectPc {
                    pc_id,
                    append: false,
                }),
                PlayerInput::host(PlayerCommand::CancelAction { pc_id }),
            ],
            SelectionCommandBatchMode::IndependentRecordedMessages,
        );

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(wait_sequence, 0)
                .expect("interrupted wait remains inspectable")
                .state,
            crate::sequence::SequenceState::Interrupted,
            "the SelectPc restitution must run before the independent cancel"
        );
        assert_eq!(
            engine
                .get_entity(pc_id)
                .and_then(Entity::pc_data)
                .expect("test PC data")
                .current_action,
            Action::NoAction,
            "the following independent cancel remains authoritative"
        );
    }

    #[test]
    fn replay_sound_boundary_consumes_prior_npc_before_current_select_bark() {
        use crate::sound::{ExclamationGroup, PendingExclamation, ResolvedExclamation};

        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        engine.control.sim_config.amount_of_speaking = 9;
        let mut input = InputState::default();
        let mut display = HostDisplayState::default();
        let npc_profile = 0x4651_0000;

        // This request belongs to the preceding engine frame. The Original
        // host resolves it after frame recording, so its trace event appears on
        // the following record before that record's selection command runs.
        engine
            .feedback
            .sound_sim
            .pending_exclamations
            .push(PendingExclamation {
                actor_id: 191,
                group: ExclamationGroup::Civilian,
                profile_id: npc_profile,
                exclamation_id: 62,
                variant: -1,
            });
        engine.queue_replay_resolved_exclamations(vec![ResolvedExclamation {
            actor_id: 191,
            identifier: npc_profile | 62,
            exclamation_id: 62,
            duration_frames: 24,
        }]);

        engine
            .hourglass_phase_sound_boundary(sim, &assets)
            .expect("replay sound boundary");
        engine.apply_commands(
            sim,
            &mut display,
            &mut input,
            &assets,
            &[PlayerInput::host(PlayerCommand::SelectPc {
                pc_id,
                append: false,
            })],
        );

        // `perform_hourglass` enters the same helper again. With the replay
        // resolutions already drained, that second entry must not consume the
        // bark queued by this boundary's input; Original will first expose it
        // to the host sound manager after the engine frame is recorded.
        engine
            .hourglass_phase_sound_boundary(sim, &assets)
            .expect("live sound boundary");

        assert_eq!(
            engine
                .feedback
                .sound_sim
                .playing_exclamations
                .iter()
                .map(|playing| (playing.actor_id, playing.exclamation_id))
                .collect::<Vec<_>>(),
            vec![(191, 62)]
        );
        assert!(engine.feedback.sound_sim.resolved_exclamations.is_empty());
        assert!(
            !engine
                .feedback
                .sound_sim
                .replay_injected_resolved_exclamations
        );
        assert_eq!(
            engine
                .feedback
                .sound_sim
                .pending_exclamations
                .iter()
                .map(|pending| (pending.actor_id, pending.exclamation_id))
                .collect::<Vec<_>>(),
            vec![(pc_id.index(), crate::engine::melee::HERO_SELECT)],
            "the current input bark must follow the preceding host sound boundary"
        );
    }

    #[test]
    fn lone_select_pc_still_restitutes_bow_action() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::player_command::{PlayerCommand, PlayerInput};
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Bow, 10)]);
        let mut input = InputState::default();
        let mut display = HostDisplayState::default();
        engine
            .get_entity_mut(pc_id)
            .and_then(Entity::pc_data_mut)
            .expect("test PC data")
            .current_action = Action::Bow;

        engine.apply_commands(
            sim,
            &mut display,
            &mut input,
            &assets,
            &[PlayerInput::host(PlayerCommand::SelectPc {
                pc_id,
                append: false,
            })],
        );

        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| sequence.elements.iter())
                .any(|element| {
                    element.owner == Some(pc_id) && element.command == Command::EquipBow
                }),
            "a live/lone SelectPc must still replay its stored Bow action"
        );
    }

    #[test]
    fn disconnect_then_reconnect_preserves_selection() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
        let (mut engine, assets, _pc_id) = setup_pc_engine(&[]);
        let mut input = InputState::default();
        let mut display = HostDisplayState::default();

        // Connect seat 2, give it a fake selection, disconnect, reconnect.
        engine.apply_commands(
            sim,
            &mut display,
            &mut input,
            &assets,
            &[PlayerInput::host(PlayerCommand::ConnectSeat {
                player_id: PlayerId(2),
                nickname: "bob".into(),
            })],
        );
        engine.players.seats[2].selection = vec![
            EntityId::Pc(crate::entity_id::PcId(7)),
            EntityId::Pc(crate::entity_id::PcId(8)),
        ];

        engine.apply_commands(
            sim,
            &mut display,
            &mut input,
            &assets,
            &[PlayerInput::host(PlayerCommand::DisconnectSeat {
                player_id: PlayerId(2),
            })],
        );
        let seat2 = engine.seat(PlayerId(2)).unwrap();
        assert!(!seat2.connected);
        assert_eq!(
            seat2.selection,
            vec![
                EntityId::Pc(crate::entity_id::PcId(7)),
                EntityId::Pc(crate::entity_id::PcId(8))
            ],
            "selection must survive disconnect"
        );

        engine.apply_commands(
            sim,
            &mut display,
            &mut input,
            &assets,
            &[PlayerInput::host(PlayerCommand::ConnectSeat {
                player_id: PlayerId(2),
                nickname: "bob_v2".into(),
            })],
        );
        let seat2 = engine.seat(PlayerId(2)).unwrap();
        assert!(seat2.connected);
        assert_eq!(seat2.nickname, "bob_v2");
        assert_eq!(
            seat2.selection,
            vec![
                EntityId::Pc(crate::entity_id::PcId(7)),
                EntityId::Pc(crate::entity_id::PcId(8))
            ]
        );
    }

    #[test]
    fn set_lock_alt_targets_issuing_seat() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
        let (mut engine, assets, _pc_id) = setup_pc_engine(&[]);
        let mut input = InputState::default();
        let mut display = HostDisplayState::default();

        // Bring up peer 2 then have it toggle alt-lock — host seat
        // must be unaffected.
        engine.apply_commands(
            sim,
            &mut display,
            &mut input,
            &assets,
            &[
                PlayerInput::host(PlayerCommand::ConnectSeat {
                    player_id: PlayerId(2),
                    nickname: "alice".into(),
                }),
                PlayerInput::new(PlayerId(2), PlayerCommand::SetLockAlt(true)),
            ],
        );
        assert!(!engine.players.seats[0].is_lock_alt, "host seat untouched");
        assert!(engine.players.seats[2].is_lock_alt, "peer 2 alt-lock on");

        // Host toggles its own alt-lock — peer 2 stays on.
        engine.apply_commands(
            sim,
            &mut display,
            &mut input,
            &assets,
            &[PlayerInput::host(PlayerCommand::SetLockAlt(true))],
        );
        assert!(engine.players.seats[0].is_lock_alt);
        assert!(engine.players.seats[2].is_lock_alt);
    }

    #[test]
    fn active_seats_skips_disconnected_peers() {
        let sim_context = crate::sim_rng::test_context();
        let sim = &sim_context;
        use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
        let (mut engine, assets, _pc_id) = setup_pc_engine(&[]);
        let mut input = InputState::default();
        let mut display = HostDisplayState::default();

        engine.apply_commands(
            sim,
            &mut display,
            &mut input,
            &assets,
            &[
                PlayerInput::host(PlayerCommand::ConnectSeat {
                    player_id: PlayerId(1),
                    nickname: "p1".into(),
                }),
                PlayerInput::host(PlayerCommand::ConnectSeat {
                    player_id: PlayerId(2),
                    nickname: "p2".into(),
                }),
                PlayerInput::host(PlayerCommand::DisconnectSeat {
                    player_id: PlayerId(1),
                }),
            ],
        );

        let active: Vec<u8> = engine.active_seats().map(|(p, _)| p.0).collect();
        // host (always) + connected peer 2; disconnected peer 1 is skipped.
        assert_eq!(active, vec![0, 2]);
    }

    fn record_interaction_quick_action(
        engine: &mut EngineInner,
        pc: EntityId,
        target: EntityId,
        command: Command,
    ) -> crate::titbit::TitbitId {
        let target_entity = engine.get_entity(target).expect("QA target exists");
        let target_position = target_entity.element_data().position_map();
        let target_layer = target_entity.element_data().layer();
        let state = engine.players.macro_store.get_or_insert(pc);
        state.begin_recording(0);
        state.append_if_recording(QuickActionStep {
            action: Action::NoAction,
            position: target_position,
            replay: QaReplayCommand::Interaction {
                target,
                command,
                double_click: false,
            },
        });
        state.stop_recording();
        let titbit = engine.feedback.titbit_manager.add_titbit(
            WorldPoint3D::new(target_position.x, target_position.y, 0.0),
            target_layer,
            TitbitKind::QuickAction,
            ElementHandle(target.index()),
            QuickAction::Default as u16,
            ElementHandle(pc.index()),
            false,
            INVALID_ID,
            true,
            None,
            Some(target_layer),
        );
        let titbit = titbit.expect("QA titbit allocation succeeds");
        engine
            .players
            .macro_store
            .get_mut(pc)
            .expect("QA owner retains macro state")
            .set_slot_titbit(0, titbit);
        titbit
    }

    fn assert_invalid_quick_action_fizzles_without_consuming(
        engine: &mut EngineInner,
        assets: &LevelAssets,
        pc: EntityId,
        titbit: crate::titbit::TitbitId,
    ) {
        let slot_before = engine
            .players
            .macro_store
            .get(pc)
            .and_then(|state| state.slot(0))
            .expect("recorded QA slot exists")
            .clone();
        let titbit_phase = engine.feedback.titbit_manager.get_phase(titbit);

        start_macro(engine, assets, pc);

        assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
        let state = engine
            .players
            .macro_store
            .get(pc)
            .expect("fizzled QA owner retains macro state");
        assert_eq!(state.slot(0), Some(&slot_before));
        assert_eq!(state.get_slot_titbit(0), Some(titbit));
        assert_eq!(
            engine.feedback.titbit_manager.get_phase(titbit),
            titbit_phase
        );
        assert!(matches!(
            engine.feedback.pending_side_effects.sounds.last(),
            Some(crate::engine::SoundCommand::Jingle(
                crate::sound::Jingle::QuickActionFailed
            ))
        ));
    }

    fn quick_action_slot_is_valid(
        engine: &EngineInner,
        assets: &LevelAssets,
        pc: EntityId,
    ) -> bool {
        let Some(steps) = engine
            .players
            .macro_store
            .get(pc)
            .and_then(|state| state.slot(0))
            .map(|slot| slot.steps.as_slice())
        else {
            return false;
        };
        engine.check_quick_action_steps_validity(assets, pc, steps)
    }

    fn configure_valid_bow_quick_action(
        engine: &mut EngineInner,
        assets: &mut LevelAssets,
        pc: EntityId,
        target: EntityId,
    ) {
        use crate::coordinates::{SpriteFrameOffset, SpriteLocalPoint};
        use crate::profiles::{BowProfile, BowShootMode};
        use crate::sprite_script::NONANIMATION_END;

        engine.mission_domain.campaign.characters[0]
            .status
            .num_arrows = 10;
        let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
        profiles.characters[0].shooting_weapon_id = 1;
        profiles.characters[0].shooting = 100;
        profiles.bows.push(BowProfile {
            normal_shoot: BowShootMode {
                range: 2000,
                ..BowShootMode::default()
            },
            ..BowProfile::default()
        });

        let action = crate::order::OrderType::ShootingWithBow;
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[action as usize] = 0;
        engine.get_entity_mut(pc).unwrap().element_data_mut().sprite = Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );

        let target_position = engine
            .get_entity(target)
            .expect("bow target exists")
            .element_data()
            .position_map();
        let target = engine.get_entity_mut(target).expect("bow target exists");
        target
            .pc_data_mut()
            .expect("bow validity fixture target is a PC")
            .life_points = 100;
        target.element_data_mut().set_position(WorldPoint3D::new(
            target_position.x,
            target_position.y,
            0.0,
        ));
    }

    #[test]
    fn quick_action_hit_and_strangle_recheck_allocated_target_state() {
        for command in [Command::HitCmd, Command::StrangleCmd] {
            let (mut engine, assets, pc, target) = setup_strangle_command_scene();
            let Entity::Soldier(soldier) = engine.get_entity_mut(target).unwrap() else {
                unreachable!("fixture target changed kind")
            };
            soldier.npc.life_points = 100;

            let titbit = record_interaction_quick_action(&mut engine, pc, target, command);
            assert!(quick_action_slot_is_valid(&engine, &assets, pc));

            engine
                .get_entity_mut(target)
                .unwrap()
                .human_data_mut()
                .unwrap()
                .unconscious = true;
            assert!(!quick_action_slot_is_valid(&engine, &assets, pc));
            assert_invalid_quick_action_fizzles_without_consuming(&mut engine, &assets, pc, titbit);
        }
    }

    #[test]
    fn quick_action_take_rechecks_allocated_object_state() {
        let (mut engine, assets, pc) = setup_pc_engine(&[(Action::Bow, 4)]);
        let target = spawn_bonus(&mut engine, ObjectType::BonusArrow, true, Action::Bow);
        let titbit = record_interaction_quick_action(&mut engine, pc, target, Command::Take);
        assert!(quick_action_slot_is_valid(&engine, &assets, pc));

        engine
            .get_entity_mut(target)
            .unwrap()
            .element_data_mut()
            .active = false;
        assert!(!quick_action_slot_is_valid(&engine, &assets, pc));
        assert_invalid_quick_action_fizzles_without_consuming(&mut engine, &assets, pc, titbit);
    }

    #[test]
    fn quick_action_search_rechecks_nested_post_seek_target_state() {
        let (mut engine, assets, pc) = setup_pc_engine(&[(Action::Search, 0)]);
        let mut target = ActorSoldier {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element.active = true;
                initial_element
            },
            actor: ActorData::default(),
            human: HumanData {
                unconscious: true,
                ..HumanData::default()
            },
            npc: NpcData::default(),
            soldier: SoldierData {
                cached_camp: Camp::Lacklandists,
                ..SoldierData::default()
            },
        };
        target
            .element
            .set_position_map(crate::coordinates::MapPoint::new(500.0, 0.0));
        let target = engine.add_entity(Entity::Soldier(target));
        let titbit = record_interaction_quick_action(&mut engine, pc, target, Command::SearchCmd);
        assert!(quick_action_slot_is_valid(&engine, &assets, pc));

        engine
            .get_entity_mut(target)
            .unwrap()
            .element_data_mut()
            .active = false;
        assert!(!quick_action_slot_is_valid(&engine, &assets, pc));
        assert_invalid_quick_action_fizzles_without_consuming(&mut engine, &assets, pc, titbit);
    }

    #[test]
    fn quick_action_bow_rechecks_allocated_target_and_owner_state() {
        let (mut engine, mut assets, pc) = setup_pc_engine(&[(Action::Bow, 10)]);
        let target = spawn_pc_at(&mut engine, 1000.0, 0.0);
        configure_valid_bow_quick_action(&mut engine, &mut assets, pc, target);

        let titbit = record_interaction_quick_action(&mut engine, pc, target, Command::ShootBow);
        assert!(quick_action_slot_is_valid(&engine, &assets, pc));

        engine
            .get_entity_mut(target)
            .unwrap()
            .element_data_mut()
            .blipped = true;
        assert!(!quick_action_slot_is_valid(&engine, &assets, pc));
        assert_invalid_quick_action_fizzles_without_consuming(&mut engine, &assets, pc, titbit);

        // The same original-game branch also rechecks the recorded owner. Keep an
        // independently recorded valid control, then invalidate only the PC.
        let (mut engine, mut assets, pc) = setup_pc_engine(&[(Action::Bow, 10)]);
        let target = spawn_pc_at(&mut engine, 1000.0, 0.0);
        configure_valid_bow_quick_action(&mut engine, &mut assets, pc, target);
        let titbit = record_interaction_quick_action(&mut engine, pc, target, Command::ShootBow);
        assert!(quick_action_slot_is_valid(&engine, &assets, pc));
        engine
            .get_entity_mut(pc)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .unconscious = true;
        assert!(!quick_action_slot_is_valid(&engine, &assets, pc));
        assert_invalid_quick_action_fizzles_without_consuming(&mut engine, &assets, pc, titbit);
    }
}
