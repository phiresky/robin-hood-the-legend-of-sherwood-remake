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
        // Keep routing exhaustive here. Family helpers only handle admitted
        // commands, and there is deliberately no shared epilogue: a family's
        // early return must finish this command without any later mutation.
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
            SelectAction { .. }
            | SelectResolvedAction { .. }
            | SelectPlannedAction { .. }
            | SelectPlannedShieldProtected { .. }
            | CancelPlannedAction
            | CancelAction { .. }
            | UnselectAllActions
            | MouseRightDown
            | MouseRightUp => {
                self.dispatch_selection_input_command(
                    sim,
                    assets,
                    seat,
                    cmd,
                    recorded_nested_selection_action,
                );
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
            SelectPc { .. }
            | TogglePcSelection { .. }
            | UnselectPc { .. }
            | BoxSelect { .. }
            | BoxUnselect { .. }
            | SelectAllPcs
            | UnselectAllPcs
            | AssignQuickGroup { .. }
            | RecallQuickGroup { .. }
            | SelectByPortrait { .. }
            | SelectTacticalUnits { .. }
            | BoxSelectTacticalUnits { .. }
            | ClearTacticalSelection
            | PinTacticalSelection
            | UnpinTacticalGroup { .. }
            | SelectTacticalGroup { .. }
            | PageTacticalPortraits { .. } => {
                self.dispatch_selection_input_command(
                    sim,
                    assets,
                    seat,
                    cmd,
                    recorded_nested_selection_action,
                );
            }

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
            StopRecordingMacro
            | StartMacro { .. }
            | DeleteMacro { .. }
            | StartRecordingMacro { .. }
            | ChangeQaMemory { .. }
            | QueueQuickAction { .. }
            | MakeQueuedActionFast { .. } => {
                self.dispatch_quick_action_command(sim, display, assets, seat, cmd);
            }

            SetLockAlt(..) | KeyControl | KeyReleaseControl => {
                self.dispatch_selection_input_command(
                    sim,
                    assets,
                    seat,
                    cmd,
                    recorded_nested_selection_action,
                );
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
            MinimapResize { .. }
            | MinimapMouseDown { .. }
            | MinimapMouseMove { .. }
            | MinimapMouseUp { .. }
            | CenterCameraOnPoint { .. }
            | MinimapRightClick
            | MinimapToggle
            | SelectFollowElement { .. }
            | ClearNpcDoubleStatusBarFlags
            | SetAmountOfSpeaking { .. }
            | SetFixHardReactionTimes { .. }
            | SetFogOfWar { .. }
            | SetTimedMissionsEnabled { .. }
            | SetDynamicAmbienceEnabled { .. }
            | SetCombatGestureRules { .. }
            | SetUnbindingEnabled { .. }
            | SetCleanHandsNpcKillsInvalidate { .. }
            | SetReusableCloaks { .. }
            | SetItemGameplayConfig { .. }
            | SetNoiseDistractionFeedback { .. }
            | SetSherwoodTrading { .. }
            | SetDiplomacyEnabled { .. }
            | SetNpcFactionWars { .. }
            | SetDiplomacyRelationship { .. } => {
                self.dispatch_camera_control_command(assets, seat, cmd);
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

    /// Applies one already-admitted command from the selection input family.
    /// The exhaustive outer dispatcher selects this family after macro recording.
    fn dispatch_selection_input_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        seat: usize,
        cmd: &PlayerCommand,
        recorded_nested_selection_action: bool,
    ) {
        use PlayerCommand::*;
        match cmd {
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

            _ => {
                unreachable!("command routed to the wrong dispatch_selection_input_command family")
            }
        }
    }

    /// Applies one already-admitted command from the quick action family.
    /// The exhaustive outer dispatcher selects this family after macro recording.
    fn dispatch_quick_action_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        seat: usize,
        cmd: &PlayerCommand,
    ) {
        use PlayerCommand::*;
        match cmd {
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
            _ => unreachable!("command routed to the wrong dispatch_quick_action_command family"),
        }
    }

    /// Applies one already-admitted command from the camera control family.
    /// The exhaustive outer dispatcher selects this family after macro recording.
    fn dispatch_camera_control_command(
        &mut self,
        assets: &LevelAssets,
        seat: usize,
        cmd: &PlayerCommand,
    ) {
        use PlayerCommand::*;
        match cmd {
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
            _ => unreachable!("command routed to the wrong dispatch_camera_control_command family"),
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
#[path = "commands/tests.rs"]
mod tests;
