//! Player command dispatch — applies [`PlayerCommand`]s to the engine.
//!
//! This is the single entry point for all player-initiated sim mutations.
//! The input system resolves raw events into commands by reading engine
//! state immutably; this module executes them.

use super::movement::GoalShape;
use super::{EngineInner, HostDisplayState, InputState, LevelAssets};
use crate::coordinates::MapPoint;
use crate::element::{ActionState, Command, EntityId, Human as _};
use crate::player_command::{PlayerCommand, PlayerInput};
use crate::profiles::Action;
use crate::sequence::{
    Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
};
use crate::titbit::{ElementHandle, INVALID_ID, QuickAction, TitbitKind};

/// Map a PC [`Action`] to the titbit phase used by the portrait
/// macro-icon strip.
///
/// The `running` flag only matters for movement; it selects Run vs
/// Walk.  For every other action the phase is fixed by the action type.
fn action_to_quick_phase(action: Action, running: bool) -> QuickAction {
    match action {
        Action::NoAction => {
            if running {
                QuickAction::Run
            } else {
                QuickAction::Walk
            }
        }
        Action::Bow => QuickAction::BowOk,
        Action::Apple => QuickAction::Apple,
        Action::Purse => QuickAction::Purse,
        Action::Stone => QuickAction::Stone,
        Action::WaspNest => QuickAction::Wasp,
        Action::Net => QuickAction::Net,
        Action::Hit | Action::HitHard => QuickAction::Hit,
        Action::Strangle => QuickAction::Strangle,
        Action::Ale | Action::Guzzle => QuickAction::Ale,
        Action::Eat => QuickAction::Eat,
        Action::Whistle => QuickAction::Whistle,
        Action::Heal | Action::Resuscitate => QuickAction::Heal,
        Action::Lever => QuickAction::Lever,
        Action::Beggar => QuickAction::Beggar,
        Action::Listen => QuickAction::Listen,
        Action::HelpToClimb => QuickAction::HelpClimb,
        Action::Shield | Action::BigShield => QuickAction::Shield,
        Action::Search => QuickAction::Search,
        Action::Tie => QuickAction::Tie,
        Action::Execute => QuickAction::Execute,
        Action::Lockpick => QuickAction::LockPick,
        Action::Climb => QuickAction::ClimbOnShoulders,
        Action::Jump => QuickAction::JumpUp,
        Action::LittleJohnCarry | Action::FarmerCarry => QuickAction::Take,
        // Fallback for action types that don't have a dedicated icon.
        Action::Test => QuickAction::Default,
    }
}

impl EngineInner {
    /// Apply a batch of player commands for the current frame.
    /// Per-frame scroll dedupe (`frame_scrolled`) is reset at the end
    /// of `perform_hourglass` (after `tick_display_state`), not here —
    /// the live game pushes scroll commands via `apply_command`
    /// (singular) one-at-a-time during input handling, while the
    /// rollback path calls `apply_commands` in a batch; both paths
    /// must dedupe identically, and the display-state tick still needs
    /// to see which directions were pressed this frame.
    pub fn apply_commands(
        &mut self,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        commands: &[PlayerInput],
    ) {
        for inp in commands {
            let seat = self.ensure_seat(inp.player_id);
            self.apply_command_for_seat(display, input, assets, seat, &inp.command);
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
    pub fn apply_local_commands(
        &mut self,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        commands: &[PlayerCommand],
    ) {
        for cmd in commands {
            self.apply_command(display, input, assets, cmd);
        }
    }

    /// Apply a single [`PlayerCommand`] as if it came from
    /// [`crate::player_command::PlayerId::HOST`].
    ///
    /// Thin wrapper around [`Self::apply_command_for_seat`] used by
    /// the single-player input path and by tests.
    pub fn apply_command(
        &mut self,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        cmd: &PlayerCommand,
    ) {
        self.apply_command_for_seat(display, input, assets, 0, cmd);
    }

    /// Apply a single player command issued by `seat`.
    ///
    /// `seat` is the index returned by [`Self::ensure_seat`].
    /// Selection-mutating handlers index `self.players.seats[seat]` so
    /// different players don't clobber each other's selections.
    pub fn apply_command_for_seat(
        &mut self,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        seat: usize,
        cmd: &PlayerCommand,
    ) {
        use PlayerCommand::*;

        // Pre-flight reachability gate for object Take clicks.  Bail
        // early when `find_authorized_position(pc.moveBox + target.position,
        // target.layer)` fails — silently skipping *both* the macro-side
        // sequence registration and the live launch.  We gate here,
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

        // Append-while-recording hook.  Records one `QuickActionStep`
        // per sim-affecting player command addressed at the currently
        // recording PC, keyed by the resolved Action (portrait bar)
        // so the macro-icon strip can render per-step titbit frames.
        self.record_macro_step_for(seat, cmd);
        match cmd {
            Noop => {} // consumed input, no action

            // ── Movement ────────────────────────────────────────
            GroupMove {
                actors,
                destination,
                running,
                show_marker,
                goal_override,
            } => {
                self.perform_group_move(
                    assets,
                    actors,
                    *destination,
                    *running,
                    *show_marker,
                    *goal_override,
                );
                // Fire `HeroSpeaking(HERO_ACCEPT_COMMAND, 0)` for the PC
                // that just accepted the move — the "yes, milord" bark.
                // It lives outside `perform_group_move` because the engine
                // helper has no access to `LevelAssets`; this is the
                // command-dispatch entry point where the assets are in
                // scope.
                for &pc_id in actors {
                    self.hero_speaking(assets, pc_id, crate::engine::melee::HERO_ACCEPT_COMMAND);
                }
            }
            StopPc { pc_id } => {
                if let Some(entity) = self.get_entity_mut(*pc_id)
                    && let Some(actor) = entity.actor_data_mut()
                {
                    actor.clear_path();
                    actor.action_state = ActionState::Waiting;
                    actor.active_movement.clear();
                }
            }

            // ── Sequence-based interactions ──────────────────────
            LaunchInteraction {
                actor,
                target,
                command,
                running,
            } => {
                // Macro recording: if `actor` is in the recording set
                // and a slot is armed, append this interaction as a step.
                if self.players.qa_recording_for.contains(actor)
                    && let Some((pos, tgt_layer, tgt_is_pc, tgt_is_object, tgt_target_filter)) =
                        self.get_entity(*target).map(|e| {
                            let target_filter = match e {
                                crate::element::Entity::Target(t) => Some(t.target.action_filter),
                                _ => None,
                            };
                            (
                                e.element_data().position_map(),
                                e.element_data().layer(),
                                e.pc_data().is_some(),
                                matches!(
                                    e,
                                    crate::element::Entity::Bonus(_)
                                        | crate::element::Entity::Scroll(_)
                                        | crate::element::Entity::Projectile(_)
                                        | crate::element::Entity::Net(_)
                                ),
                                target_filter,
                            )
                        })
                {
                    let action = self
                        .get_entity(*actor)
                        .and_then(|e| e.pc_data())
                        .map(|pc| pc.current_action)
                        .unwrap_or(crate::profiles::Action::NoAction);
                    // Pick the QuickAction ordinal.  Priority:
                    //   1. `Command::Take` on an object target → Take.
                    //   2. FX-target interaction → walk the target's
                    //      filter ladder so levers, cut/handle/take
                    //      targets, pay-targets, bow targets, etc. pick
                    //      the per-filter icon instead of the
                    //      action-bar default.
                    //   3. Action-specific icon when the PC is in an
                    //      armed action mode (bow, stone, etc.).
                    //   4. Fallback `InteractPc` / `InteractNpc`.
                    let fallback_quick = if tgt_is_pc {
                        crate::titbit::QuickAction::InteractPc as u16
                    } else {
                        crate::titbit::QuickAction::InteractNpc as u16
                    };
                    let quick = if *command == Command::Take && tgt_is_object {
                        crate::titbit::QuickAction::Take as u16
                    } else if let Some(filter) = tgt_target_filter {
                        let pc_char_profile = self
                            .get_entity(*actor)
                            .and_then(|e| e.pc_data())
                            .and_then(|pc| assets.profile_manager.get_character(pc.profile_index));
                        let pc_has_search = pc_char_profile
                            .is_some_and(|p| p.has_contextual_action(Action::Search));
                        let pc_is_vip = self
                            .get_entity(*actor)
                            .is_some_and(|e| self.is_entity_vip(assets, e));
                        super::target_interaction::target_qa_titbit(
                            filter,
                            pc_has_search,
                            pc_is_vip,
                        )
                    } else {
                        crate::macro_store::action_to_qa_frame(action).unwrap_or(fallback_quick)
                    };
                    // Drop any titbit still sitting in this QA slot before
                    // we allocate a new one.
                    let slot = self.players.qa_recording_slot;
                    self.remove_quick_action_titbits_for(*actor, slot);
                    // Register a QuickAction titbit on the target so
                    // the renderer can look it up by id.
                    let tgt_handle = crate::titbit::ElementHandle(target.index());
                    let pc_handle = crate::titbit::ElementHandle(actor.index());
                    let titbit_id = self.feedback.titbit_manager.add_titbit(
                        crate::coordinates::WorldPoint3D {
                            x: pos.x,
                            y: pos.y,
                            z: 0.0,
                        },
                        tgt_layer,
                        crate::titbit::TitbitKind::QuickAction,
                        tgt_handle,
                        quick,
                        pc_handle,
                        false,
                        crate::titbit::INVALID_ID,
                        true,
                        None,
                        Some(tgt_layer),
                    );
                    // Write the new titbit id into the slot.  Only
                    // overwrite when the titbit manager returned a real
                    // id, to avoid clobbering with INVALID.
                    if let Some(tb) = crate::titbit::TitbitId::new(titbit_id) {
                        self.players
                            .macro_store
                            .get_or_insert(*actor)
                            .set_slot_titbit(slot as usize, tb);
                    }
                    // NOTE: the QuickActionStep is appended by the shared
                    // `record_macro_step_for` helper which ran at the top
                    // of `apply_command`; no append here to avoid
                    // duplicating the dotted-chain step.
                }
                self.apply_interaction_with_seek(*actor, *target, *command, *running);
            }
            LaunchGroundTarget {
                actor,
                target_pos,
                command,
                target_field,
                titbit_layer,
            } => {
                if self.players.qa_recording_for.contains(actor) {
                    let action = self
                        .get_entity(*actor)
                        .and_then(|e| e.pc_data().map(|pc| pc.current_action))
                        .unwrap_or(crate::profiles::Action::NoAction);
                    // Ground-target moves: Run icon for running
                    // animations, Walk otherwise.  We don't have the
                    // animation here yet, so default to Walk;
                    // action-specific icons win when the PC is acting
                    // with a known Action.
                    let quick = crate::macro_store::action_to_qa_frame(action)
                        .unwrap_or(crate::titbit::QuickAction::Walk as u16);
                    // Drop any titbit still sitting in this QA slot.
                    let slot = self.players.qa_recording_slot;
                    self.remove_quick_action_titbits_for(*actor, slot);
                    let pc_handle = crate::titbit::ElementHandle(actor.index());
                    // The titbit position and per-action layer (Net=0,
                    // Wasp/Purse = selected layer) arrive pre-resolved
                    // on the `PlayerCommand` so the handler just forwards.
                    let titbit_pos = crate::coordinates::WorldPoint3D {
                        x: target_pos.x,
                        y: target_pos.y,
                        z: target_pos.z,
                    };
                    let titbit_id = self.feedback.titbit_manager.add_titbit(
                        titbit_pos,
                        *titbit_layer,
                        crate::titbit::TitbitKind::QuickAction,
                        crate::titbit::ElementHandle::INVALID,
                        quick,
                        pc_handle,
                        false,
                        crate::titbit::INVALID_ID,
                        true,
                        None,
                        Some(*titbit_layer),
                    );
                    // Write the new titbit id into the slot.  Skip INVALID.
                    if let Some(tb) = crate::titbit::TitbitId::new(titbit_id) {
                        self.players
                            .macro_store
                            .get_or_insert(*actor)
                            .set_slot_titbit(slot as usize, tb);
                    }
                    // QuickActionStep appended by `record_macro_step_for`
                    // at the top of `apply_command`.
                }
                let mut elem = SequenceElement::new_generic(1, *command, Some(*actor));
                // The sequence field is the full 3D throw target (the
                // downstream `ThrowNet/Purse/WaspNest` tick arms read
                // the x/y and drop z, so the Point3D variant stays
                // compatible while preserving the true altitude for
                // any future consumer).
                elem.set_property(
                    *target_field,
                    FieldValue::Point3D {
                        x: target_pos.x,
                        y: target_pos.y,
                        z: target_pos.z,
                    },
                );
                self.launch_element(elem);
            }
            LaunchSelfAbility { actor, command } => {
                let elem = SequenceElement::new(1, *command, Some(*actor));
                self.launch_element(elem);
            }
            LaunchScrollRead {
                actor,
                target,
                running,
            } => {
                if self.players.qa_recording_for.contains(actor) {
                    let Some(pos) = self
                        .get_entity(*target)
                        .map(|e| e.element_data().position_map())
                    else {
                        return;
                    };
                    let slot = self.players.qa_recording_slot;
                    self.remove_quick_action_titbits_for(*actor, slot);
                    let pc_handle = crate::titbit::ElementHandle(actor.index());
                    let target_layer = self
                        .get_entity(*target)
                        .map(|e| e.element_data().layer())
                        .unwrap_or(0);
                    let titbit_id = self.feedback.titbit_manager.add_titbit(
                        crate::coordinates::WorldPoint3D {
                            x: pos.x,
                            y: pos.y,
                            z: 0.0,
                        },
                        target_layer,
                        crate::titbit::TitbitKind::QuickAction,
                        crate::titbit::ElementHandle(target.index()),
                        crate::titbit::QuickAction::Search as u16,
                        pc_handle,
                        false,
                        crate::titbit::INVALID_ID,
                        true,
                        None,
                        Some(target_layer),
                    );
                    if let Some(tb) = crate::titbit::TitbitId::new(titbit_id) {
                        self.players
                            .macro_store
                            .get_or_insert(*actor)
                            .set_slot_titbit(slot as usize, tb);
                    }
                }
                self.apply_scroll_read_with_seek(*actor, *target, *running);
            }

            // ── Swordfight ──────────────────────────────────────
            EnterSwordfight {
                actor,
                target,
                running,
            } => {
                self.apply_enter_swordfight(assets, *actor, *target, *running);
            }
            SwordStrikeCmd {
                actor,
                target,
                command,
                with_seek,
            } => {
                tracing::trace!(
                    ?actor,
                    ?target,
                    ?command,
                    with_seek,
                    "PlayerCommand::SwordStrikeCmd"
                );
                if *with_seek {
                    self.apply_sword_strike_with_seek(assets, *actor, *target, *command);
                } else {
                    let elem =
                        SequenceElement::new_interaction(1, *command, Some(*actor), Some(*target));
                    self.launch_element(elem);
                }
            }
            SetPrincipalOpponent { actor, opponent_id } => {
                self.set_as_new_principal_opponent(assets, *actor, *opponent_id);
            }

            // ── Action bar ──────────────────────────────────────
            SelectAction {
                pc_id,
                action_index,
            } => {
                self.select_pc_action_by_index(assets, input, seat, *pc_id, *action_index as u8);
            }
            CancelAction { pc_id } => {
                self.set_pc_action(
                    assets,
                    input,
                    seat,
                    *pc_id,
                    crate::profiles::Action::NoAction,
                );
            }
            UnselectAllActions => {
                for pc_id in self.players.seats[seat].selection.clone() {
                    self.unselect_action(pc_id);
                }
            }
            MouseRightDown => {
                input.right_mouse_down = true;
            }
            MouseRightUp => {
                input.right_mouse_down = false;
            }
            ClearShootList { pc_id } => {
                // Drop the queued shoot list — pending
                // `Command::ShootBow` elements in `elements_to_go`.
                let resolver = Self::priority_resolver(&self.entities);
                self.sequence_manager.stop_pending_elements_matching(
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
                let mut elem = SequenceElement::new_generic(1, Command::DropAmmo, Some(*pc_id));
                elem.set_property(Field::ActionId, FieldValue::Integer(*action_id));
                elem.set_property(Field::Amount, FieldValue::Integer(*amount));
                self.launch_element(elem);
            }
            DropAleAt {
                actor,
                target_pos,
                running,
            } => {
                self.apply_drop_ale_at(*actor, *target_pos, *running);
            }
            ShieldSelectProtected {
                actor: _,
                protected_pc,
            } => {
                // Stash the focused PC as the shield protectee and
                // flip `is_protected = false` so the next click resolves
                // the danger point.  No sequence is launched.
                self.shield.protected_pc = Some(*protected_pc);
                self.shield.is_protected = false;
            }
            RaiseShieldWithDanger {
                actor,
                protected_pc,
                danger_point,
                danger_point_layer,
            } => {
                self.apply_raise_shield_with_danger(
                    *actor,
                    *protected_pc,
                    crate::coordinates::MapPoint {
                        x: danger_point.x,
                        y: danger_point.y,
                    },
                    *danger_point_layer,
                );
            }

            // ── Posture ─────────────────────────────────────────
            CrouchDown => self.apply_crouch_down(seat),
            StandUp => self.apply_stand_up(seat),

            // ── Selection ───────────────────────────────────────
            SelectPc { pc_id, append } => {
                self.select_pc(assets, seat, *pc_id, *append, true);
                self.update_recording_after_selection_change();
            }
            TogglePcSelection { pc_id } => {
                self.toggle_pc_selection(assets, seat, *pc_id);
                self.update_recording_after_selection_change();
            }
            BoxSelect { pt1, pt2, shift } => {
                self.apply_box_select(assets, input, seat, *pt1, *pt2, *shift);
                self.update_recording_after_selection_change();
            }
            BoxUnselect { pt1, pt2 } => {
                self.apply_box_unselect(input, seat, *pt1, *pt2);
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
                // Portrait click → `select_by_portrait_index` fires
                // `select_pc` with `speak=true` directly.
                self.select_by_portrait_index(assets, seat, *portrait_index as u8, *append);
                self.update_recording_after_selection_change();
            }

            // ── Special ─────────────────────────────────────────
            ResetComa { pc_id } => self.reset_coma(assets, *pc_id),
            SendReinforcement { pc_id } => self.request_reinforcement(*pc_id),
            // Use the actor-level MakeFast so the pathfinder + queued
            // transitions get rewritten, not just the element-level
            // action.
            MakePcFast { pc_id } => self.actor_make_fast(*pc_id),
            MakePcSlow { pc_id } => self.actor_make_slow(*pc_id),
            MakePcUpright { pc_id } => self.actor_make_upright(*pc_id),
            MakePcCrouched { pc_id } => self.actor_make_crouched(*pc_id),

            ChangeState(req) => {
                self.change_state_with_camera_display(seat, *req);
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
                self.apply_start_macro(display, input, assets, *pc, *slot);
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
            SetLockAlt(on) => {
                self.players.seats[seat].is_lock_alt = *on;
            }
            KeyControl => {
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
            }
            #[cfg(not(target_os = "macos"))]
            KeyReleaseControl => {
                // Restore each selected PC's saved action.  Stored
                // per-PC on `PcData::saved_action`, so different
                // selections regain different actions.
                let ids = self.players.seats[seat].selection.clone();
                for id in ids {
                    let (saved, cur) = match self.get_entity(id).and_then(|e| e.pc_data()) {
                        Some(pc) => (pc.saved_action, pc.current_action),
                        None => continue,
                    };
                    if cur != saved {
                        self.unselect_action(id);
                    }
                    if let Some(entity) = self.get_entity_mut(id)
                        && let Some(pc) = entity.pc_data_mut()
                    {
                        pc.current_action = saved;
                    }
                }
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
                self.dispatch_startup_message(assets, *msg, *arg1, *arg2);
            }
            RefreshSelectedPatchDisplayDoors { selected_patch_idx } => {
                self.refresh_selected_patch_display_doors(*selected_patch_idx);
            }
            RevealAllBlips => {
                self.reveal_all_blips();
            }
            CampaignSelectNextMission { mission_idx } => {
                if let Some(campaign) = self.mission_domain.campaign.as_mut() {
                    campaign.select_next_mission(*mission_idx, &assets.profile_manager);
                }
            }
            CampaignSwapPendingToAccessibleMissions => {
                if let Some(campaign) = self.mission_domain.campaign.as_mut() {
                    campaign.swap_pending_to_accessible_missions();
                }
            }
            CampaignHarvestProductionSectorState => {
                self.harvest_production_sector_state(assets);
            }
            CampaignConvertSelectedPeasantsToBlazons => {
                self.convert_selected_peasants_to_blazons(&assets.profile_manager);
            }
            ApplyQuitMissionUpdates { exit_code } => {
                self.apply_quit_mission_updates(assets, *exit_code);
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
                let sw = screen.x;
                let sh = screen.y;
                display
                    .minimap
                    .set_widget_position(*base, *corner_size, sw, sh);
            }
            MinimapMouseDown { click_pt } => {
                // Begin dragging on LEFTDOWN inside widget when the map
                // is deployed.
                if display.minimap.is_displayed() {
                    let screen = Self::director_camera_view_size();
                    let sw = screen.x;
                    let sh = screen.y;
                    let was_dragging = display.minimap.drag_start();
                    display.minimap.manage_dragging(*click_pt, sw, sh);
                    // Fire UiHasFocus on the second ManageDragging call
                    // (the continuing-drag arm).  The first call only
                    // records the anchor.
                    if was_dragging {
                        self.messenger.send(crate::messenger::Message::new(
                            crate::messenger::MessageType::Simple(
                                crate::messenger::SimpleMessage::UiHasFocus,
                            ),
                        ));
                        // The messenger broadcast is consumed on the
                        // next tick; flipping `input.has_focus` to
                        // false synchronously here suppresses every
                        // mouse dispatch for the remainder of this
                        // frame too.
                        input.has_focus = false;
                    }
                }
            }
            MinimapMouseMove {
                mouse_pt,
                left_mouse_down,
            } => {
                // Hover state.
                let over_widget = display.minimap.is_over_widget(*mouse_pt);
                if over_widget {
                    if !*left_mouse_down {
                        display.minimap.ui_state = crate::minimap::UIState::Focused;
                        display.minimap.entered_nicely = true;
                    }
                    display.minimap.capture = true;
                } else if !display.minimap.drag_start {
                    display.minimap.entered_nicely = false;
                    display.minimap.ui_state = crate::minimap::UIState::Default;
                    display.minimap.capture = false;
                }

                // Drag continuation: check drag_start before the
                // inside-widget test so drags continue even when the
                // cursor leaves the widget.
                if *left_mouse_down && display.minimap.drag_start {
                    let screen = Self::director_camera_view_size();
                    let sw = screen.x;
                    let sh = screen.y;
                    display.minimap.manage_dragging(*mouse_pt, sw, sh);
                    // Continuing-drag branch forwards UiHasFocus every
                    // frame to suppress edge-scrolling and hide the
                    // PC-info popup.  Both halves: enqueue for the tick
                    // drain (hides the PC-info popup next tick) and
                    // clear `input.has_focus` synchronously so the rest
                    // of this frame's mouse events skip dispatch.
                    self.messenger.send(crate::messenger::Message::new(
                        crate::messenger::MessageType::Simple(
                            crate::messenger::SimpleMessage::UiHasFocus,
                        ),
                    ));
                    input.has_focus = false;
                }
            }
            MinimapMouseUp {
                click_pt,
                on_minimap,
            } => {
                // Check the dragged flag, dead zone, and dispatch to
                // open-map or center-on-click.
                display.minimap.drag_start = false;
                if !display.minimap.dragged {
                    if *on_minimap {
                        if !display.minimap.is_displayed() {
                            display.minimap.manage_click();
                        } else {
                            let usable = crate::minimap::usable_area(&display.minimap.map_box);
                            let level_size = self.feedback.cutscene_camera.level_size;
                            let world_pt = display.minimap.map_to_real(*click_pt, level_size);
                            if usable.contains_point(*click_pt)
                                && let Some(world_pt) = world_pt
                            {
                                // Gate the recenter on `is_zoom_possible`
                                // and clear the gameplay locker via
                                // `LockerOff` before centering — skip
                                // both when a zoom is in flight so the
                                // click can't jank the view.
                                if self.is_zoom_possible(display) {
                                    // Minimap recenter is host-driven UI;
                                    // toggles the host seat's locker.
                                    self.change_state(
                                        display,
                                        0,
                                        crate::engine::EngineStateRequest::LockerOff,
                                    );
                                    self.center_on_point(0, world_pt);
                                }
                            }
                        }
                    }
                } else {
                    display.minimap.dragged = false;
                }
                display.minimap.close_after_highlight = false;
            }
            MinimapRightClick => {
                // Unconditional close animation start (no
                // transition_counter guard) so a right-click during the
                // opening animation immediately reverses to closing.
                display.minimap.force_close_animation();
                display.minimap.highlighted_elements.clear();
            }
            MinimapToggle => {
                // Open if hidden, close if shown.  Both arms set the
                // counters unconditionally so an in-flight transition
                // reverses immediately, and the close arm also flips
                // the UI state to Selected.
                if display.minimap.is_displayed() {
                    display.minimap.force_close_animation();
                } else {
                    display.minimap.force_open_animation();
                }
            }

            // ── Display / UI setters ────────────────────────────
            SelectFollowElement { entity_id } => {
                self.select_follow_element(seat, *entity_id);
            }
            ClearNpcDoubleStatusBarFlags => {
                self.clear_npc_double_status_bar_flags();
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
                let idx = self.ensure_seat(*target);
                let was_connected = self.players.seats[idx].connected;
                self.players.seats[idx].connected = true;
                self.players.seats[idx].nickname = nickname.clone();
                if was_connected {
                    tracing::info!(
                        player_id = ?target,
                        nickname = %nickname,
                        "seat reconnected (nickname updated)"
                    );
                } else {
                    tracing::info!(
                        player_id = ?target,
                        nickname = %nickname,
                        "seat connected"
                    );
                }
            }
            DisconnectSeat { player_id: target } => {
                let idx = target.0 as usize;
                if let Some(s) = self.players.seats.get_mut(idx) {
                    if s.connected {
                        tracing::info!(
                            player_id = ?target,
                            nickname = %s.nickname,
                            selection_size = s.selection.len(),
                            "seat disconnected (selection preserved)"
                        );
                    }
                    s.connected = false;
                } else {
                    tracing::debug!(
                        player_id = ?target,
                        "DisconnectSeat for unknown seat — ignored"
                    );
                }
            }
        }

        // Persist the deployed minimap top-left to the active player
        // profile on every accepted move.  Drain the per-tick dirty
        // flag here so any minimap command (drag, resize-revalidate)
        // emits a single side effect for the host to persist.
        if let Some(top_left) = display.minimap.take_pending_position() {
            self.feedback.pending_side_effects.pending_minimap_position = Some(top_left);
        }
    }

    /// Append a `QuickActionStep` to the currently-recording PC's
    /// macro, if a recording is in progress and the command targets
    /// that PC.  No-op otherwise.
    ///
    /// Only the `Action` + target `position` is stored per step — the
    /// per-slot titbit id is set separately at the `AddTitbit` site.
    fn record_macro_step_for(&mut self, seat: usize, cmd: &PlayerCommand) {
        if self.players.qa_recording_for.is_empty() {
            return;
        }
        // When multiple PCs are armed for recording, each one receives
        // its own macro step.  Snapshot the set up-front so we can
        // re-borrow `self` inside the per-PC loop.
        let recording_pcs = self.players.qa_recording_for.clone();
        for recording_pc in recording_pcs {
            self.record_macro_step_for_pc(seat, cmd, recording_pc);
        }
    }

    fn record_macro_step_for_pc(
        &mut self,
        seat: usize,
        cmd: &PlayerCommand,
        recording_pc: EntityId,
    ) {
        use crate::macro_store::QuickActionStep;
        use PlayerCommand::*;

        // Helper: read the acting PC's current action.  Returns
        // NoAction if the entity isn't a PC or doesn't exist.
        let pc_action = |engine: &EngineInner, pc: EntityId| -> crate::profiles::Action {
            engine
                .get_entity(pc)
                .and_then(|e| e.pc_data())
                .map(|pc| pc.current_action)
                .unwrap_or(crate::profiles::Action::NoAction)
        };

        let entity_pos = |engine: &EngineInner, id: EntityId| -> Option<MapPoint> {
            engine
                .get_entity(id)
                .map(|e| e.element_data().position_map())
        };

        // Track whether this command is a running move (selects Run
        // vs Walk titbit phase).
        let mut running_move = false;
        // Override the `action`-derived slot-titbit phase — used by
        // commands whose recorded phase isn't a function of the PC's
        // `current_action` (e.g. posture toggles, which record Down /
        // Up regardless of what action is currently armed).
        let mut phase_override: Option<crate::titbit::QuickAction> = None;
        use crate::macro_store::QaReplayCommand;
        let (actor, action, position, replay): (
            EntityId,
            crate::profiles::Action,
            MapPoint,
            QaReplayCommand,
        ) = match cmd {
            GroupMove {
                actors,
                destination,
                running,
                show_marker: _,
                goal_override: _,
            } => {
                if !actors.contains(&recording_pc) {
                    return;
                }
                running_move = *running;
                (
                    recording_pc,
                    crate::profiles::Action::NoAction, // move → Walk/Run titbit path
                    *destination,
                    QaReplayCommand::Move {
                        destination: *destination,
                        running: *running,
                    },
                )
            }
            LaunchInteraction {
                actor,
                target,
                command,
                running,
            } => {
                if *actor != recording_pc {
                    return;
                }
                let Some(target_pos) = entity_pos(self, *target) else {
                    return;
                };
                let action = pc_action(self, *actor);
                running_move = *running;
                (
                    *actor,
                    action,
                    target_pos,
                    QaReplayCommand::Interaction {
                        target: *target,
                        command: *command,
                        // The double-click bit is the same bit that
                        // drives `running=true` on the input side, so
                        // reuse it as our recorded double-click flag.
                        double_click: *running,
                    },
                )
            }
            LaunchGroundTarget {
                actor,
                target_pos,
                command,
                target_field,
                titbit_layer,
            } => {
                if *actor != recording_pc {
                    return;
                }
                let action = pc_action(self, *actor);
                let pos = MapPoint::new(target_pos.x, target_pos.y);
                (
                    *actor,
                    action,
                    pos,
                    QaReplayCommand::GroundTarget {
                        target_pos: *target_pos,
                        command: *command,
                        target_field: *target_field,
                        titbit_layer: *titbit_layer,
                    },
                )
            }
            DropAleAt {
                actor,
                target_pos,
                running,
            } => {
                if *actor != recording_pc {
                    return;
                }
                running_move = *running;
                let pos = *target_pos;
                (
                    *actor,
                    crate::profiles::Action::Ale,
                    pos,
                    QaReplayCommand::DropAle {
                        target_pos: *target_pos,
                        running: *running,
                    },
                )
            }
            LaunchSelfAbility { actor, command } => {
                if *actor != recording_pc {
                    return;
                }
                let Some(pos) = entity_pos(self, *actor) else {
                    return;
                };
                let action = pc_action(self, *actor);
                (
                    *actor,
                    action,
                    pos,
                    QaReplayCommand::SelfAbility { command: *command },
                )
            }
            LaunchScrollRead {
                actor,
                target,
                running,
            } => {
                if *actor != recording_pc {
                    return;
                }
                let Some(pos) = entity_pos(self, *target) else {
                    return;
                };
                running_move = *running;
                (
                    *actor,
                    crate::profiles::Action::Search,
                    pos,
                    QaReplayCommand::ScrollRead {
                        target: *target,
                        running: *running,
                    },
                )
            }
            EnterSwordfight {
                actor,
                target,
                running,
            } => {
                if *actor != recording_pc {
                    return;
                }
                let Some(pos) = entity_pos(self, *target) else {
                    return;
                };
                // The macro-strip icon for an enter-swordfight click
                // is the dedicated swordfight glyph, not the action's
                // default phase.
                phase_override = Some(crate::titbit::QuickAction::SwordFight);
                (
                    *actor,
                    crate::profiles::Action::Hit,
                    pos,
                    QaReplayCommand::Swordfight {
                        target: *target,
                        running: *running,
                    },
                )
            }
            SwordStrikeCmd {
                actor,
                target,
                command,
                with_seek,
            } => {
                if *actor != recording_pc {
                    return;
                }
                let Some(pos) = entity_pos(self, *target) else {
                    return;
                };
                (
                    *actor,
                    crate::profiles::Action::Hit,
                    pos,
                    QaReplayCommand::SwordStrike {
                        target: *target,
                        command: *command,
                        with_seek: *with_seek,
                    },
                )
            }
            CrouchDown | StandUp => {
                // For each selected PC, we either perform the live
                // posture change or register a posture-toggle step
                // into the macro slot.  This helper runs once per
                // recording PC; emit the step only when that PC is
                // also in the current selection — the non-recording
                // selection members continue to fall through to the
                // live apply path in `apply_crouch_down` /
                // `apply_stand_up`.
                if !self.players.seats[seat].selection.contains(&recording_pc) {
                    return;
                }
                let Some(pos) = entity_pos(self, recording_pc) else {
                    return;
                };
                let to_crouch = matches!(cmd, CrouchDown);
                phase_override = Some(if to_crouch {
                    crate::titbit::QuickAction::Down
                } else {
                    crate::titbit::QuickAction::Up
                });
                (
                    recording_pc,
                    crate::profiles::Action::NoAction,
                    pos,
                    QaReplayCommand::PostureToggle { to_crouch },
                )
            }
            // The remaining commands are UI / selection and don't push
            // into the macro recording.
            _ => return,
        };

        self.players.macro_store.append(
            actor,
            QuickActionStep {
                action,
                position,
                replay,
            },
        );

        // Register a QuickAction titbit once per macro slot and feed
        // the id into the slot.
        let Some(pc_state) = self.players.macro_store.get(recording_pc) else {
            return;
        };
        let Some(slot_idx) = pc_state.recording_slot() else {
            return;
        };
        if pc_state.get_slot_titbit(slot_idx as usize).is_some() {
            return;
        }
        let phase = match phase_override {
            Some(q) => q as u16,
            None => action_to_quick_phase(action, running_move) as u16,
        };
        let layer = self
            .get_entity(recording_pc)
            .map(|e| e.element_data().layer())
            .unwrap_or(0);
        let pos3d = crate::coordinates::WorldPoint3D {
            x: position.x,
            y: position.y,
            z: 0.0,
        };
        let manager = ElementHandle(recording_pc.index());
        let titbit_id = self.feedback.titbit_manager.add_titbit(
            pos3d,
            layer,
            TitbitKind::QuickAction,
            ElementHandle::INVALID,
            phase,
            manager,
            running_move, // Run companion titbit
            INVALID_ID,
            true,
            None,
            Some(layer),
        );
        if let Some(tb) = crate::titbit::TitbitId::new(titbit_id)
            && let Some(pc_state) = self.players.macro_store.get_mut(recording_pc)
        {
            pc_state.set_slot_titbit(slot_idx as usize, tb);
        }
    }

    /// Play back macro slot `slot` on `pc` (or on every PC with one at
    /// `slot` when `pc` is `None`).
    ///
    /// For each PC with a macro in the slot, the recorded steps are
    /// re-dispatched in order through `apply_command`, producing the
    /// same effects as the original live inputs.  Then the slot's
    /// titbit is removed and the slot is cleared.  If every PC that
    /// had a macro in that slot has now had it fire,
    /// [`EngineInner::do_tetris_macro`] collapses the slot so slot
    /// `N+1` shifts down.
    ///
    /// Recording is stopped first.
    fn apply_start_macro(
        &mut self,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        pc: Option<EntityId>,
        slot: u8,
    ) {
        // Stop any in-flight recording.
        self.stop_recording_macro();

        let targets: Vec<EntityId> = match pc {
            Some(id) => {
                if self.has_quick_action(id, slot) {
                    vec![id]
                } else {
                    Vec::new()
                }
            }
            None => self
                .pc_ids
                .iter()
                .copied()
                .filter(|id| self.has_quick_action(*id, slot))
                .collect(),
        };

        if targets.is_empty() {
            return;
        }

        for pc_id in &targets {
            self.replay_macro_slot(display, input, assets, *pc_id, slot);
        }

        // When at least one PC tried to launch a macro, jingle either
        // QuickActionSucceeded (every target consumed its slot) or
        // QuickActionFailed (some target still has the slot — its
        // sequence build refused).  `targets.is_empty()` was checked
        // above so at-least-one-launched is implicitly true here.
        let all_launched = !targets.iter().any(|id| self.has_quick_action(*id, slot));
        let jingle = if all_launched {
            crate::sound::Jingle::QuickActionSucceeded
        } else {
            crate::sound::Jingle::QuickActionFailed
        };
        self.feedback
            .pending_side_effects
            .sounds
            .push(super::SoundCommand::Jingle(jingle));

        // If this was an "all PCs" launch and every PC that had a macro
        // at this slot has now fired (i.e. no PC still has one), collapse
        // the strip.
        if pc.is_none() && all_launched {
            self.do_tetris_macro(display, slot);
        }
    }

    /// Replay one PC's macro slot — the per-PC half of [`apply_start_macro`].
    /// Extracted so the iteration above can re-borrow `self` between steps.
    fn replay_macro_slot(
        &mut self,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        pc: EntityId,
        slot: u8,
    ) {
        // Pre-flight: if any recorded element fails its per-element
        // gate, the entire macro is rejected and the slot is preserved
        // so the player can retry.  The replay walks per-step rather
        // than rebuilding one sequence, so we run the gate once up
        // front and bail without dispatching or clearing on failure —
        // the jingle path in `apply_start_macro` then keys off the
        // slot still being occupied to emit `QuickActionFailed`.
        if !self.check_quick_action_validity(pc, slot) {
            return;
        }

        // Snapshot the steps — replay must not be perturbed by any
        // macro-store mutation the dispatched commands perform (the
        // recording-append gate runs inside `apply_command`, but
        // `stop_recording_macro` was called in `apply_start_macro` so
        // `qa_recording_for` is None and no appends will happen).
        let steps: Vec<crate::macro_store::QuickActionStep> = self
            .players
            .macro_store
            .get(pc)
            .map(|s| {
                s.slot(slot as usize)
                    .map(|slot| slot.steps.clone())
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        for step in steps {
            let cmd = match step.replay {
                crate::macro_store::QaReplayCommand::Move {
                    destination,
                    running,
                } => PlayerCommand::GroupMove {
                    actors: vec![pc],
                    destination,
                    running,
                    show_marker: true,
                    // Macro replay always re-resolves via spatial lookup;
                    // patch redirects only fire from the live click path.
                    goal_override: None,
                },
                crate::macro_store::QaReplayCommand::Interaction {
                    target,
                    command,
                    double_click,
                } => {
                    // Runtime second-line-of-defence for the per-step
                    // validity gate.  `check_quick_action_validity`
                    // already pre-flighted missing-target steps, but a
                    // step earlier in the replay can have removed the
                    // target since.  Whole-sequence abort: bail out
                    // without clearing the slot or launching posture
                    // recovery, so the slot survives and
                    // `apply_start_macro`'s `has_quick_action` check
                    // fires `QuickActionFailed`.
                    if self.get_entity(target).is_none() {
                        return;
                    }
                    // When the recorded button was a double-click,
                    // dispatch a leading single-click before the
                    // recorded click — the sim advances each step
                    // inline so back-to-back dispatches achieve the
                    // "single primes, double commits" sequencing.
                    if double_click {
                        let pre_click = PlayerCommand::LaunchInteraction {
                            actor: pc,
                            target,
                            command,
                            running: false,
                        };
                        self.apply_command(display, input, assets, &pre_click);
                    }
                    PlayerCommand::LaunchInteraction {
                        actor: pc,
                        target,
                        command,
                        // QA replay clones recorded elements verbatim;
                        // the live Run flag is captured per-step by
                        // the titbit and replayed via `MakeFast` on
                        // the PC before the clone.  Keep
                        // `running=false` here to match the
                        // conservative `WalkingUpright` picked by the
                        // seek fallback; the real `MakeFast` path
                        // still fires via `actor_make_fast` callers.
                        running: false,
                    }
                }
                crate::macro_store::QaReplayCommand::ScrollRead { target, running } => {
                    // See Interaction arm — whole-sequence abort on
                    // target-gone.
                    if self.get_entity(target).is_none() {
                        return;
                    }
                    PlayerCommand::LaunchScrollRead {
                        actor: pc,
                        target,
                        running,
                    }
                }
                crate::macro_store::QaReplayCommand::GroundTarget {
                    target_pos,
                    command,
                    target_field,
                    titbit_layer,
                } => PlayerCommand::LaunchGroundTarget {
                    actor: pc,
                    target_pos,
                    command,
                    target_field,
                    titbit_layer,
                },
                crate::macro_store::QaReplayCommand::SelfAbility { command } => {
                    PlayerCommand::LaunchSelfAbility { actor: pc, command }
                }
                crate::macro_store::QaReplayCommand::DropAle {
                    target_pos,
                    running,
                } => PlayerCommand::DropAleAt {
                    actor: pc,
                    target_pos,
                    running,
                },
                crate::macro_store::QaReplayCommand::Swordfight { target, running } => {
                    // See Interaction arm — whole-sequence abort on
                    // target-gone.
                    if self.get_entity(target).is_none() {
                        return;
                    }
                    PlayerCommand::EnterSwordfight {
                        actor: pc,
                        target,
                        running,
                    }
                }
                crate::macro_store::QaReplayCommand::SwordStrike {
                    target,
                    command,
                    with_seek,
                } => {
                    // See Interaction arm — whole-sequence abort on
                    // target-gone.
                    if self.get_entity(target).is_none() {
                        return;
                    }
                    PlayerCommand::SwordStrikeCmd {
                        actor: pc,
                        target,
                        command,
                        with_seek,
                    }
                }
                crate::macro_store::QaReplayCommand::PostureToggle { to_crouch } => {
                    // Replay a recorded `CrouchDown` / `StandUp` on
                    // the macro's owning PC.  The existing
                    // `CrouchDown` / `StandUp` dispatch targets the
                    // whole selection, so we route through the per-PC
                    // actor helpers instead to keep the replay scoped
                    // to a single PC.
                    if to_crouch {
                        self.actor_make_crouched(pc);
                    } else {
                        let posture = self
                            .get_entity(pc)
                            .map(|e| e.element_data().posture)
                            .unwrap_or(crate::element::Posture::Upright);
                        match posture {
                            crate::element::Posture::Crouched => {
                                self.actor_make_upright(pc);
                            }
                            crate::element::Posture::SimulatingBeggar => {
                                let elem = SequenceElement::new(1, Command::LeaveBeggar, Some(pc));
                                self.launch_element(elem);
                            }
                            crate::element::Posture::Spy
                            | crate::element::Posture::AnonymousArcher => {
                                let elem = SequenceElement::new(1, Command::LeaveSpy, Some(pc));
                                self.launch_element(elem);
                            }
                            crate::element::Posture::Tree => {
                                let elem = SequenceElement::new(1, Command::LeaveTree, Some(pc));
                                self.launch_element(elem);
                            }
                            _ => {}
                        }
                    }
                    continue;
                }
            };
            self.apply_command(display, input, assets, &cmd);
        }

        // Tack a posture-restoration element (EquipBow / CrouchDown /
        // EnterHelpingClimb / EnterBeggar) onto the end of the macro.
        // The replay dispatches each recorded step through
        // `apply_command` rather than building one big sequence, so
        // recovery lands in two places:
        //   * Move-tailed macros — `perform_group_move` already calls
        //     `append_posture_recovery` on the move's launched
        //     sequence (movement.rs:738/855/1940), embedding recovery
        //     into the move's post-seek.
        //   * Non-Move-tailed macros (Interaction / SwordStrike /
        //     SelfAbility / etc.) — those apply paths don't add
        //     recovery themselves, so launch a standalone recovery
        //     element here.  Calling `append_posture_recovery` with an
        //     empty Sequence skips the function's "last-was-SEEK →
        //     attach to post-seek" branch (no last element to inspect)
        //     and produces a single bare element keyed off the PC's
        //     current posture / action_state — which is the right
        //     element to launch into the actor's queue post-replay.
        let mut recovery = crate::sequence::Sequence::default();
        self.append_posture_recovery(pc, &mut recovery);
        for element in recovery.elements {
            self.launch_element(element);
        }

        // Post-seek continuation is now ported via
        // `ActorData::post_seek_sequence`: seek-building helpers attach
        // their continuation directly to the launched movement element,
        // so replay does not need an extra per-PC handoff here.

        // Drop the slot's titbit and clear the slot.
        self.remove_quick_action_titbits_for(pc, slot);
        if let Some(state) = self.players.macro_store.get_mut(pc) {
            state.clear_slot(slot as usize);
        }
    }

    /// Pre-flight validity gate for QA replay:
    ///
    ///   * empty slot → fail;
    ///   * any step references a target entity that no longer exists →
    ///     fail;
    ///   * any non-MOVE/SEEK/POSTURE step while the PC is currently
    ///     swordfighting → fail.  `Move` (which expands to MOVE/SEEK
    ///     on dispatch) and `PostureToggle` survive the gate (the
    ///     posture quickitos has no swordfight restriction); recorded
    ///     interactions, sword-strikes, abilities, ground-targets,
    ///     etc. all fail.
    ///
    /// Returns `true` to allow replay, `false` to fizzle.
    fn check_quick_action_validity(&self, pc: EntityId, slot: u8) -> bool {
        use crate::macro_store::QaReplayCommand;
        let Some(state) = self.players.macro_store.get(pc) else {
            return false;
        };
        let Some(slot_data) = state.slot(slot as usize) else {
            return false;
        };
        if slot_data.steps.is_empty() {
            return false;
        }
        let is_swordfighting = self
            .get_entity(pc)
            .and_then(|e| e.human_data())
            .map(|h| !h.opponents.is_empty())
            .unwrap_or(false);
        for step in &slot_data.steps {
            let target = match &step.replay {
                QaReplayCommand::Interaction { target, .. }
                | QaReplayCommand::ScrollRead { target, .. }
                | QaReplayCommand::Swordfight { target, .. }
                | QaReplayCommand::SwordStrike { target, .. } => Some(target),
                _ => None,
            };
            if let Some(target) = target
                && self.get_entity(*target).is_none()
            {
                return false;
            }
            // Per-element swordfight gate: while the PC is mid-fight,
            // only MOVE, SEEK, or PostureToggle may run.  `Move`
            // covers MOVE+SEEK on dispatch; `PostureToggle` enters
            // through the quickitos path which has no swordfight
            // gate, so it must also pass.
            if is_swordfighting
                && !matches!(
                    step.replay,
                    QaReplayCommand::Move { .. } | QaReplayCommand::PostureToggle { .. }
                )
            {
                return false;
            }
        }
        true
    }

    /// Begin recording a macro.  `pc = None` arms on every
    /// currently-selected PC; `pc = Some(id)` targets that specific
    /// PC's portrait directly.
    fn apply_start_recording_macro(&mut self, seat: usize, pc: Option<EntityId>, slot: u8) {
        if (slot as usize) >= crate::macro_store::NUMBER_OF_QA_MEMORY {
            return;
        }
        let targets: Vec<EntityId> = match pc {
            Some(id) => vec![id],
            None => self.players.seats[seat].selection.clone(),
        };
        if targets.is_empty() {
            return;
        }
        for id in &targets {
            self.players
                .macro_store
                .get_or_insert(*id)
                .begin_recording(slot);
        }
        self.players.qa_recording_slot = slot;
        self.players.qa_recording_for = targets;
    }

    /// Swap the active recording slot on the selected PCs.  Ends
    /// recording on the old slot, then begins recording on the new
    /// slot — both operate on the *currently-selected* set, not the
    /// set that was previously recording.
    fn apply_change_qa_memory(&mut self, seat: usize, slot: u8) {
        if (slot as usize) >= crate::macro_store::NUMBER_OF_QA_MEMORY {
            return;
        }
        // End recording on every PC that was armed (the currently-
        // armed set, not the current selection — those can differ).
        let old = std::mem::take(&mut self.players.qa_recording_for);
        for id in &old {
            if let Some(state) = self.players.macro_store.get_mut(*id) {
                state.stop_recording();
            }
        }
        // Re-arm on whoever is currently selected.
        let targets: Vec<EntityId> = self.players.seats[seat].selection.clone();
        if targets.is_empty() {
            return;
        }
        for id in &targets {
            self.players
                .macro_store
                .get_or_insert(*id)
                .begin_recording(slot);
        }
        self.players.qa_recording_slot = slot;
        self.players.qa_recording_for = targets;
    }

    /// Drop macro slot `slot` without replaying.
    ///
    /// For "all PCs" deletion, also fire the tetris collapse so the
    /// strip closes up.  Single-PC deletion does not tetris.
    fn apply_delete_macro(
        &mut self,
        display: &mut HostDisplayState,
        pc: Option<EntityId>,
        slot: u8,
    ) {
        self.stop_recording_macro();
        match pc {
            Some(id) => {
                self.abort_quick_action(id, slot);
            }
            None => {
                let pcs = self.pc_ids.clone();
                for id in pcs {
                    self.abort_quick_action(id, slot);
                }
                self.do_tetris_macro(display, slot);
            }
        }
    }

    // ── Complex command helpers ──────────────────────────────────

    /// Is `target` an object-class entity whose click routes through
    /// the `find_authorized_position` pre-flight?
    ///
    /// Matches the Bonus / Scroll / Projectile / Net arms of
    /// `object_pickup_command`.
    fn is_object_take_target(&self, target: EntityId) -> bool {
        matches!(
            self.get_entity(target),
            Some(
                crate::element::Entity::Bonus(_)
                    | crate::element::Entity::Scroll(_)
                    | crate::element::Entity::Projectile(_)
                    | crate::element::Entity::Net(_)
            )
        )
    }

    /// Pre-flight reachability check for object Take clicks.
    ///
    /// Translates the PC's move-box to the target's map position and
    /// calls `find_authorized_position` with the target's layer.  A
    /// `false` return tells the caller to silently no-op — neither
    /// launching the seek sequence nor installing the QA titbit.
    fn object_take_reachable(&self, actor: EntityId, target: EntityId) -> bool {
        let Some(actor_entity) = self.get_entity(actor) else {
            return false;
        };
        let Some(target_entity) = self.get_entity(target) else {
            return false;
        };
        let move_box = actor_entity.position_iface().get_move_box();
        if !move_box.is_somewhere() {
            return false;
        }
        let tgt_pos = target_entity.position_iface().map_position();
        let tgt_layer = target_entity.element_data().layer();
        let mut box_at_target = move_box.translated(tgt_pos);
        self.fast_grid
            .find_authorized_position(&mut box_at_target, tgt_layer)
    }

    fn actor_action_distance(
        &self,
        actor: EntityId,
        animation: crate::order::OrderType,
    ) -> Option<f32> {
        let Some(entity) = self.get_entity(actor) else {
            tracing::warn!(
                ?actor,
                ?animation,
                "actor_action_distance: actor entity is missing"
            );
            return None;
        };
        match entity.sprite().action_distance(animation) {
            Ok(distance) => Some(distance),
            Err(err) => {
                tracing::warn!(
                    ?actor,
                    ?animation,
                    error = %err,
                    "actor_action_distance: missing sprite action distance"
                );
                None
            }
        }
    }

    fn interaction_action_distance(&self, actor: EntityId, command: Command) -> Option<f32> {
        match command_action_distance_animation(command) {
            Some(animation) => self.actor_action_distance(actor, animation),
            None => Some(interaction_distance(command)),
        }
    }

    /// Launch an interaction, prepending a Seek walk if the actor is
    /// too far away or in a different sector.
    fn apply_interaction_with_seek(
        &mut self,
        actor: EntityId,
        target: EntityId,
        command: Command,
        running: bool,
    ) {
        // Ranged actions bypass the seek entirely: the actor fires or
        // throws from wherever it stands.  This mirrors the original
        // bow click path, which launches RHCOMMAND_SHOOT_BOW directly.
        if matches!(
            command,
            Command::ShootBow | Command::ShootBowOnce | Command::ThrowApple | Command::ThrowStone
        ) {
            let elem = SequenceElement::new_interaction(1, command, Some(actor), Some(target));
            self.launch_element(elem);
            return;
        }

        // ClimbUpOnShoulders has a multi-element post-seek that the
        // generic single-interaction path can't express:
        // `Seek(USE_POINT, tolerance=8) → [TurnElement(L1) →
        // ClimbUpOnShoulders(L2)]`.  Route through a dedicated helper.
        if command == Command::ClimbUpOnShoulders {
            self.apply_climb_on_shoulders_with_seek(actor, target, running);
            return;
        }

        // When the click was a double-click and the PC is *not*
        // recording a macro, just call `MakeFast()` and drop the
        // freshly built interaction outright — the double-click is
        // treated as an "accelerate the current order" gesture, not
        // as a queue of a new running interaction.  Only applies to
        // the seek-with-interaction commands listed below.
        let is_addinteraction_with_seek_command = matches!(
            command,
            Command::StrangleCmd
                | Command::HitCmd
                | Command::HealCmd
                | Command::Pay
                | Command::SearchCmd
                | Command::SwordstrikeDown
                | Command::TieCmd
                | Command::WakeUp
                | Command::UseLever
                | Command::Take
        );
        let is_recording_macro = self
            .players
            .macro_store
            .get(actor)
            .map(|s| s.is_recording())
            .unwrap_or(false);
        if running && !is_recording_macro && is_addinteraction_with_seek_command {
            self.actor_make_fast(actor);
            return;
        }

        // Suppress the beggar's alms-request remarks during the
        // entire seek + receive-purse animation chain by stamping the
        // don't-talk counter at click time, before the walk-up starts.
        // `reveal_scrolls` bumps the same counter again at the chain's
        // end, so both sites are needed.
        if command == Command::Pay
            && let Some(crate::element::Entity::Civilian(c)) = self.get_entity_mut(target)
            && let crate::element::AiBrain::Friendly(ref mut ai) = c.npc.ai_brain
        {
            ai.set_beggar_dont_talk_counter(3);
        }

        let (pc_pos, pc_sector, pc_posture) = match self.get_entity(actor) {
            Some(e) => (
                e.element_data().position_map(),
                e.element_data().sector(),
                e.element_data().posture,
            ),
            None => return,
        };
        // When `b_use_action_point` is set, the gating distance check
        // uses the antagonist's action-point (sprite hotspot of the
        // current row) — the position the PC will actually face/touch
        // on arrival — instead of the antagonist's map centre.  The
        // only command that uses this is Pay, so the beggar's
        // right-hand position governs whether the PC needs to walk in.
        // Note: only the gating destination changes — the seek
        // movement itself still targets the entity, and the
        // face-opponent USE_POINT flag (already on for Pay) lines
        // on-arrival positioning up with the same hotspot.
        let b_use_action_point = command == Command::Pay;
        let (tgt_pos, tgt_sector, take_tolerance_override) = match self.get_entity(target) {
            Some(e) => {
                let pos_map = e.element_data().position_map();
                let gating_pos = if b_use_action_point {
                    match e.sprite().current_hotspot() {
                        Some(hp) => {
                            let ps = e.cxx_position_sprite();
                            crate::coordinates::MapPoint {
                                x: ps.x + hp.x,
                                y: ps.y + hp.y,
                            }
                        }
                        None => pos_map,
                    }
                } else {
                    pos_map
                };
                (
                    gating_pos,
                    e.element_data().sector(),
                    (command == Command::Take).then(|| take_seek_tolerance(e)),
                )
            }
            None => return,
        };
        // Per-object Take tolerance is `radius + 15` — non-trivial
        // for Purse (22), Coin (18) and Net (25 crumpled / 55
        // uncrumpled).  Fall back to the default table for every
        // other command.
        let action_distance = match take_tolerance_override {
            Some(distance) => distance,
            None => match self.interaction_action_distance(actor, command) {
                Some(distance) => distance,
                None => return,
            },
        };

        let dx = pc_pos.x - tgt_pos.x;
        let dy = pc_pos.y - tgt_pos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        let same_sector = pc_sector.is_some() && pc_sector == tgt_sector;

        // Per-command move flags:
        //   Strangle, Hit → NO_TRANSITIONS | SEEK_STOP_NPC
        //   Heal / Search / SwordstrikeDown / Tie / Take →
        //     SEEK_IN_BUILDINGS
        // `NO_TRANSITIONS` suppresses the stand↔crouch retry the seek
        // would otherwise inject; `SEEK_STOP_NPC` asks the victim NPC
        // to halt on arrival; `SEEK_IN_BUILDINGS` lets `RefreshSeek`
        // short-circuit when both actor and target are already inside
        // the same building.
        let mut per_command_seek_flags = MoveFlags::empty();
        match command {
            Command::StrangleCmd | Command::HitCmd => {
                per_command_seek_flags |= MoveFlags::NO_TRANSITIONS | MoveFlags::SEEK_STOP_NPC;
            }
            Command::HealCmd
            | Command::SearchCmd
            | Command::SwordstrikeDown
            | Command::TieCmd
            | Command::Take => {
                per_command_seek_flags |= MoveFlags::SEEK_IN_BUILDINGS;
            }
            _ => {}
        }

        let needs_seek = dist > action_distance || !same_sector;
        tracing::trace!(
            ?actor,
            ?target,
            ?command,
            dist,
            action_distance,
            same_sector,
            needs_seek,
            "apply_interaction_with_seek"
        );

        let mut interaction = SequenceElement::new_interaction(
            if needs_seek { 2 } else { 1 },
            command,
            Some(actor),
            Some(target),
        );

        if needs_seek {
            // Pick the seek animation from `running` (double-click) +
            // posture:
            //   running=true → RunningUpright (even when crouched —
            //     the MakeFast/animation pipeline stands up first).
            //   running=false + crouched → WalkingCrouched
            //   running=false + upright  → WalkingUpright
            let action_style = if running {
                crate::order::OrderType::RunningUpright
            } else if pc_posture == crate::element::Posture::Crouched {
                crate::order::OrderType::WalkingCrouched
            } else {
                crate::order::OrderType::WalkingUpright
            };
            let mut seek =
                SequenceElement::new_movement(1, Command::Seek, Some(actor), action_style);
            if let SequenceElementData::Movement {
                element,
                tolerance,
                flags,
                ..
            } = &mut seek.data
            {
                *element = Some(target);
                *tolerance = action_distance;
                *flags |= MoveFlags::SEEK | per_command_seek_flags;
                // Net is the only seek target that uses
                // `DIRECTIONAL_TOLERANCE`.  When the target is a
                // landed net, set it — the tolerance check projects
                // onto the seek direction so the PC can stop slightly
                // to the side of the net sprite instead of needing to
                // be exactly within radius.
                if command == Command::Take
                    && matches!(
                        self.get_entity(target),
                        Some(crate::element::Entity::Net(_))
                    )
                {
                    *flags |= MoveFlags::DIRECTIONAL_TOLERANCE;
                }
            }

            // SEEK_STOP_NPC is consumed by `resolve_entity_seek` at
            // initial dispatch / RefreshSeek time, where the chase
            // speed and distance gates are available.

            interaction.command_level = 1;
            let mut post_seek = Sequence::new();
            post_seek.append_element(interaction);
            if let SequenceElementData::Movement {
                post_seek_sequence, ..
            } = &mut seek.data
            {
                *post_seek_sequence = Some(Box::new(post_seek));
            }

            let mut seq = Sequence::new();
            seq.append_element(seek);
            self.launch_sequence(seq);
        } else {
            self.launch_element(interaction);
        }
    }

    /// Fire `EVENT_STOP` on a target NPC that a PC is currently
    /// seeking with `SEEK_STOP_NPC`.  No-op when the target isn't an
    /// NPC or isn't in a moving action state.
    pub(crate) fn send_seek_stop_to_npc(&mut self, target: EntityId) {
        let Some(entity) = self.get_entity_mut(target) else {
            return;
        };
        // Moving-state precondition: only fire when the target is
        // actually in flight — the same two action states covered by
        // `ActionState::is_moving`.
        let is_moving = entity
            .actor_data()
            .is_some_and(|a| a.action_state.is_moving());
        if !is_moving {
            return;
        }
        let Some(npc) = entity.npc_data_mut() else {
            return;
        };
        let Some(base) = npc.ai_brain.base_mut() else {
            return;
        };
        base.pending_self_stimuli
            .push(crate::ai::StimulusType::EventStop);
    }

    /// Launch the scroll-read composite sequence on `pc`, prepending a
    /// Seek walk when the PC is too far from `npc`.
    ///
    /// Build and launch the scroll-read composite sequence.
    ///
    /// The inner sequence is:
    ///   level 1: `LockAi` (only when the NPC's AI isn't already
    ///                      script-locked)
    ///   level 1: `TurnElement` PC → NPC
    ///   level 1: `TurnElement` NPC → PC
    ///   level 2: `UnlockAi` (only when the LockAi above was emitted)
    ///   level 2: `OpenScroll` carrying Scroll / ScrollReader /
    ///                         ScrollOwner
    ///
    /// A `Seek` movement element is prepended when
    /// `norm(pc_pos - npc_pos) > action_distance` (= 30), the
    /// composite attaches as the post-seek payload, and the whole
    /// thing launches.  When the PC is already in range the composite
    /// launches directly.  The seek uses `USE_POINT` so the arrival
    /// faces the NPC.
    fn apply_scroll_read_with_seek(&mut self, actor: EntityId, target: EntityId, running: bool) {
        use crate::sequence::{Field, FieldValue};

        // `running && !is_recording` is a short-circuit — the PC just
        // gets `MakeFast` and we never build the composite.
        let is_recording = self.is_recording_macro();
        if running && !is_recording {
            self.actor_make_fast(actor);
            return;
        }

        let (pc_pos, pc_posture) = match self.get_entity(actor) {
            Some(e) => (e.element_data().position_map(), e.element_data().posture),
            None => return,
        };
        let (npc_pos, npc_has_scroll, npc_ai_script_locked) = match self.get_entity(target) {
            Some(e) => {
                let has_scroll = match e {
                    crate::element::Entity::Soldier(s) => s.npc.scroll_attached,
                    crate::element::Entity::Civilian(c) => c.npc.scroll_attached,
                    _ => false,
                };
                let locked = e.ai_controller().is_some_and(|ai| ai.ai_is_script_locked());
                (e.element_data().position_map(), has_scroll, locked)
            }
            None => return,
        };
        if !npc_has_scroll {
            tracing::warn!(
                ?actor,
                ?target,
                "apply_scroll_read_with_seek: target NPC is not scroll-attached"
            );
            return;
        }

        // Resolve the attached scroll entity id. The script-side
        // `scroll_attachments` map is keyed by actor script handle.
        let npc_handle = crate::natives::GameHost::actor_handle(target);
        let scroll_handle: Option<i32> = self
            .mission_script
            .as_ref()
            .and_then(|s| s.game_host())
            .and_then(|h| h.scroll_attachments.get(&npc_handle).copied());
        let Some(scroll_handle) = scroll_handle else {
            tracing::warn!(
                ?actor,
                ?target,
                "apply_scroll_read_with_seek: scroll_attachments map has no entry for NPC"
            );
            return;
        };
        let Some(scroll_id) = self.entity_id_for_actor_handle(scroll_handle) else {
            tracing::warn!(
                ?actor,
                ?target,
                scroll_handle,
                "apply_scroll_read_with_seek: invalid scroll handle"
            );
            return;
        };

        if is_recording {
            // Macro recording already installed the QA titbit and stored
            // `QaReplayCommand::ScrollRead` through the top-level
            // `record_macro_step_for` hook.  This matches the verified
            // legacy implementation path:
            //   NPC::MouseClicked -> PC::AddSequenceWithSeek ->
            //   SetQuickActionSequence, then DisableAllActionsTemp only
            //   when RHEngine::IsClimbingOrInBuilding(this) is true,
            //   followed by MSG_STOP_RECORDING_MACRO.
            //
            // The live scroll-read sequence is not launched while
            // recording; playback rebuilds it from the semantic
            // `ScrollRead` step and current engine state.
            if self.is_pc_climbing_or_in_building(actor) {
                self.apply_disable_all_actions_temp(0, Some(actor), true);
            }
            self.stop_recording_macro();
            return;
        }

        // Animation style — same decision matrix as
        // `apply_interaction_with_seek`: running overrides posture,
        // otherwise the seek inherits the PC's crouched/upright stance.
        let action_style = if running {
            crate::order::OrderType::RunningUpright
        } else if pc_posture == crate::element::Posture::Crouched {
            crate::order::OrderType::WalkingCrouched
        } else {
            crate::order::OrderType::WalkingUpright
        };

        let action_distance =
            match self.actor_action_distance(actor, crate::order::OrderType::Listening) {
                Some(distance) => distance,
                None => return,
            };

        // Build the composite command sequence.  Level numbers are
        // relative to this sequence; elements at the same level run
        // concurrently and advance together.  LockAi and both
        // TurnElements share level 1; UnlockAi / OpenScroll share
        // level 2.  The first TurnElement turns the PC toward the
        // NPC, the second turns the NPC toward the PC.
        let turn_pc =
            SequenceElement::new_interaction(1, Command::TurnElement, Some(actor), Some(target));
        let turn_npc =
            SequenceElement::new_interaction(1, Command::TurnElement, Some(target), Some(actor));

        let mut scroll_elem = SequenceElement::new_generic(2, Command::OpenScroll, None);
        scroll_elem.set_property(Field::Scroll, FieldValue::Element(scroll_id));
        scroll_elem.set_property(Field::ScrollReader, FieldValue::Element(actor));
        scroll_elem.set_property(Field::ScrollOwner, FieldValue::Element(target));

        let mut command_seq = Sequence::new();
        if !npc_ai_script_locked {
            command_seq.append_element(SequenceElement::new(1, Command::LockAi, Some(target)));
        }
        command_seq.append_element(turn_pc);
        command_seq.append_element(turn_npc);
        if !npc_ai_script_locked {
            command_seq.append_element(SequenceElement::new(2, Command::UnlockAi, Some(target)));
        }
        command_seq.append_element(scroll_elem);

        // Distance check: when the PC is already in range, launch
        // the composite directly.
        let dx = pc_pos.x - npc_pos.x;
        let dy = pc_pos.y - npc_pos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        tracing::trace!(
            ?actor,
            ?target,
            dist,
            action_distance,
            running,
            "apply_scroll_read_with_seek"
        );

        if dist <= action_distance {
            self.launch_sequence(command_seq);
            return;
        }

        // Face-opponent on arrival → USE_POINT on the seek.
        let mut seek = SequenceElement::new_movement(1, Command::Seek, Some(actor), action_style);
        if let SequenceElementData::Movement {
            element,
            tolerance,
            flags,
            post_seek_sequence,
            ..
        } = &mut seek.data
        {
            *element = Some(target);
            *tolerance = action_distance;
            *flags |= MoveFlags::SEEK | MoveFlags::USE_POINT;
            *post_seek_sequence = Some(Box::new(command_seq));
        }

        let mut seq = Sequence::new();
        seq.append_element(seek);
        self.launch_sequence(seq);
    }

    /// Build `[Seek(USE_POINT, tolerance=8) → (TurnElement(L1) →
    /// ClimbUpOnShoulders(L2))]` for a click on a HelpingToClimb PC.
    /// Skips the seek when the climber is already inside the
    /// tolerance.
    fn apply_climb_on_shoulders_with_seek(
        &mut self,
        actor: EntityId,
        target: EntityId,
        running: bool,
    ) {
        let (pc_pos, pc_posture) = match self.get_entity(actor) {
            Some(e) => (e.element_data().position_map(), e.element_data().posture),
            None => return,
        };
        let tgt_pos = match self.get_entity(target) {
            Some(e) => e.element_data().position_map(),
            None => return,
        };

        let action_distance = match self
            .actor_action_distance(actor, crate::order::OrderType::ClimbingUpOnShoulders)
        {
            Some(distance) => distance,
            None => return,
        };

        let action_style = if running {
            crate::order::OrderType::RunningUpright
        } else if pc_posture == crate::element::Posture::Crouched {
            crate::order::OrderType::WalkingCrouched
        } else {
            crate::order::OrderType::WalkingUpright
        };

        let turn =
            SequenceElement::new_interaction(1, Command::TurnElement, Some(actor), Some(target));
        let climb = SequenceElement::new_interaction(
            2,
            Command::ClimbUpOnShoulders,
            Some(actor),
            Some(target),
        );
        let mut command_seq = Sequence::new();
        command_seq.append_element(turn);
        command_seq.append_element(climb);

        let dx = pc_pos.x - tgt_pos.x;
        let dy = pc_pos.y - tgt_pos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        tracing::trace!(
            ?actor,
            ?target,
            dist,
            action_distance,
            running,
            "apply_climb_on_shoulders_with_seek"
        );

        if dist <= action_distance {
            self.launch_sequence(command_seq);
            return;
        }

        let mut seek = SequenceElement::new_movement(1, Command::Seek, Some(actor), action_style);
        if let SequenceElementData::Movement {
            element,
            tolerance,
            flags,
            post_seek_sequence,
            ..
        } = &mut seek.data
        {
            *element = Some(target);
            *tolerance = action_distance;
            *flags |= MoveFlags::SEEK | MoveFlags::USE_POINT;
            *post_seek_sequence = Some(Box::new(command_seq));
        }

        let mut seq = Sequence::new();
        seq.append_element(seek);
        self.launch_sequence(seq);
    }

    fn apply_enter_swordfight(
        &mut self,
        assets: &LevelAssets,
        pc_id: EntityId,
        target_id: EntityId,
        running: bool,
    ) {
        use crate::element::{Camp, Entity};
        use crate::order::OrderType;

        // VIP gate
        let target_is_vip = self
            .get_entity(target_id)
            .map(|e| crate::engine::melee::is_vip_from_profile(e, &assets.profile_manager))
            .unwrap_or(false);
        if target_is_vip {
            let pc_is_robin = self
                .get_entity(pc_id)
                .and_then(|e| e.pc_data())
                .is_some_and(|pc| pc.robin);
            if !pc_is_robin {
                let speak = SequenceElement::new(1, Command::SpeakVipsAreForRobin, Some(pc_id));
                self.launch_element(speak);
                return;
            }
        }

        // Status filter
        let status_ok = {
            let target = match self.get_entity(target_id) {
                Some(e) => e,
                None => return,
            };
            let is_blipped = target.element_data().blipped;
            let is_dead = target.is_dead();
            let is_unconscious = target.human_data().is_some_and(|h| h.unconscious);
            let (is_lacklandist, scroll_attached) = match target {
                Entity::Soldier(s) => (s.camp() == Camp::Lacklandists, s.npc.scroll_attached),
                _ => (false, false),
            };
            !is_blipped && !is_dead && !is_unconscious && is_lacklandist && !scroll_attached
        };

        if !status_ok {
            // Fallthrough to use-interaction.  Rewrite coin clicks
            // to the source purse before launching the Take sequence.
            if let Some(cmd) = determine_use_command(self, assets, pc_id, target_id) {
                let launch_target = if cmd == Command::Take {
                    coin_pickup_target(self, target_id)
                } else {
                    target_id
                };
                self.apply_interaction_with_seek(pc_id, launch_target, cmd, false);
            }
            return;
        }

        // When recording a macro, the swordfight sequence is
        // registered as a QA step and recording stops — the PC does
        // *not* engage the fight live.  The QA step + titbit are
        // already appended in `record_macro_step_for_pc` (called from
        // `apply_command` at the top of dispatch); short-circuit here
        // so we don't double up with a live launch, then stop the
        // recording.
        if self.players.qa_recording_for.contains(&pc_id) {
            self.stop_recording_macro();
            return;
        }

        // Animation style:
        //   single click        → WalkingUpright / WalkingCrouched
        //   dbl-click + record  → RunningUpright
        //   dbl-click + !record → MakeFast() (handled before we get
        //                                    here, via MakePcFast)
        // The PC must seek in a non-combat animation; a sword animation
        // here would force action_state = MovingSword at seek dispatch
        // (tick.rs), visually starting the fight while still out of
        // range.  EnterSwordfight flips the action state to sword mode
        // once the seek completes.
        let action_style = if running {
            OrderType::RunningUpright
        } else {
            match self.get_entity(pc_id).map(|e| e.element_data().posture) {
                Some(crate::element::Posture::Crouched) => OrderType::WalkingCrouched,
                _ => OrderType::WalkingUpright,
            }
        };

        // Table swordfight check
        if let Some(aggressor_line_idx) = crate::engine::melee::is_table_swordfight_needed(
            &self.entities,
            &self.fast_grid,
            &assets.profile_manager,
            pc_id,
            target_id,
        ) {
            self.apply_table_swordfight(pc_id, target_id, aggressor_line_idx, action_style);
            return;
        }

        // Classical seek + enter
        let seek_tolerance = self
            .get_entity(pc_id)
            .and_then(|e| crate::engine::melee::get_hth_weapon_id_full(e, &assets.profile_manager))
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .map(|p| p.distance[crate::weapons::WeaponDistance::Default as usize] as f32)
            .unwrap_or(40.0);

        // Cross-sector routing: when the target is separated from the
        // PC by one or more gates, a plain `Command::Seek` never
        // crosses them.  Route through `build_gate_movement_sequence`
        // so the actor walks through gates and then seeks the target.
        // When a swordfight jump-line pair spans the final hop, use
        // `GoalShape::Line` so the arrival check snaps to line
        // tolerance.
        let (pc_sector, pc_pos, pc_layer) = match self.get_entity(pc_id) {
            Some(e) => (
                e.element_data().sector(),
                e.element_data().position_map(),
                e.element_data().layer(),
            ),
            None => return,
        };
        let (target_sector, target_pos, target_layer) = match self.get_entity(target_id) {
            Some(e) => (
                e.element_data().sector(),
                e.element_data().position_map(),
                e.element_data().layer(),
            ),
            None => return,
        };

        if let (Some(pcs), Some(ts)) = (pc_sector, target_sector)
            && pcs != ts
        {
            // Source adaptation: when the PC is currently straddling
            // a gate, rewrite the path source to the gate's far-side
            // anchor.
            let (door_handle, door_direction) = self
                .get_entity(pc_id)
                .map(|e| e.position_iface())
                .map(|p| (p.get_door(), p.get_door_direction()))
                .unwrap_or((crate::position_interface::DoorHandle::NULL, false));
            let (adj_src_pos, adj_src_sector) = {
                let host = self.mission_script.as_mut().and_then(|s| s.game_host_mut());
                let adapted = host.and_then(|h| {
                    crate::engine::movement::adapt_source_to_current_door(
                        &h.doors,
                        door_handle,
                        door_direction,
                    )
                });
                match adapted {
                    Some((adj, sector, _layer)) => (adj, sector),
                    None => (MapPoint::new(pc_pos.x, pc_pos.y), u16::from(pcs)),
                }
            };
            // PC authorisation for the gate A*.  Seek/melee routing
            // never sets the leave-map flag, so `allow_leave_map = false`.
            let pc_auth = self.get_entity(pc_id).map(|e| e.actor_auth_info());
            let gate_path = {
                let host = self.mission_script.as_mut().and_then(|s| s.game_host_mut());
                host.and_then(|h| {
                    crate::gate::find_path_gates(
                        &h.doors,
                        (adj_src_pos.x, adj_src_pos.y),
                        adj_src_sector,
                        (target_pos.x, target_pos.y),
                        ts.into(),
                        pc_auth.as_ref(),
                        false,
                        &|sector| {
                            h.sector_kinds
                                .get(&u16::from(sector))
                                .and_then(|k| k.lift_type)
                        },
                    )
                })
            };
            // Detect a swordfight-line pair between the PC's sector
            // and the target's sector — the "across gates" snap case.
            // Computed regardless of whether `find_path_gates`
            // succeeded so the fallback branches below can also use a
            // line-arrival on it.
            let swordfight_line = crate::engine::melee::table_swordfight_jump_line(
                &self.fast_grid,
                i16::from(pcs),
                i16::from(ts),
                target_pos,
                seek_tolerance,
            );
            let swordfight_line_idx =
                swordfight_line.and_then(crate::jump_line::JumpLineIndex::new);

            let path_failed = gate_path.is_none();
            if let Some(path) = gate_path
                && !path.is_empty()
            {
                let (goal_shape, arrival_layer) = if let Some(aggr_idx) = swordfight_line_idx
                    && let Some(jl) = self.fast_grid.level.jump_lines.get(usize::from(aggr_idx))
                {
                    let mid = jl.get_middle_point();
                    (
                        GoalShape::Line {
                            line_index: aggr_idx,
                            midpoint: mid,
                            tolerance: seek_tolerance,
                        },
                        jl.layer,
                    )
                } else {
                    (
                        GoalShape::Point(MapPoint::new(target_pos.x, target_pos.y)),
                        target_layer,
                    )
                };

                self.build_gate_movement_sequence(
                    pc_id,
                    path,
                    goal_shape,
                    arrival_layer,
                    action_style,
                    true,
                    1.0,
                    MoveFlags::empty(),
                    Vec::new(),
                    Vec::new(),
                    true,
                    true,
                );

                let mut enter_elem =
                    SequenceElement::new_generic(2, Command::EnterSwordfight, Some(pc_id));
                enter_elem.set_property(Field::Opponent, FieldValue::Element(target_id));
                enter_elem.set_property(
                    Field::JumplineDestination,
                    match swordfight_line_idx {
                        Some(idx) => FieldValue::LineId(idx),
                        None => FieldValue::Integer(0),
                    },
                );
                self.launch_element(enter_elem);
                return;
            }

            // No usable gate path.  When `find_path_gates` fails for
            // a PC, fire `HeroSpeaking(HERO_UNABLE_TO_DO_SOMETHING)`
            // before bailing.  Then, if a swordfight jump-line was
            // detected, emit a single `Move` with `MoveFlags::LINE` +
            // `line_id` to the line midpoint.  Falls through to the
            // classical Seek + EnterSwordfight when no line is set.
            if path_failed {
                self.hero_speaking(
                    assets,
                    pc_id,
                    crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                );
            }
            if let Some(aggr_idx) = swordfight_line_idx
                && let Some(jl) = self.fast_grid.level.jump_lines.get(usize::from(aggr_idx))
            {
                let mid = jl.get_middle_point();
                let arrival_layer = jl.layer;
                let mut move_elem =
                    SequenceElement::new_movement(1, Command::Move, Some(pc_id), action_style);
                if let SequenceElementData::Movement {
                    destination,
                    layer,
                    tolerance,
                    flags,
                    line_id,
                    ..
                } = &mut move_elem.data
                {
                    *destination = crate::coordinates::MapPoint { x: mid.x, y: mid.y };
                    *layer = arrival_layer;
                    *tolerance = seek_tolerance;
                    *flags |= MoveFlags::LINE;
                    *line_id = Some(aggr_idx);
                }

                let mut enter_elem =
                    SequenceElement::new_generic(2, Command::EnterSwordfight, Some(pc_id));
                enter_elem.set_property(Field::Opponent, FieldValue::Element(target_id));
                enter_elem.set_property(Field::JumplineDestination, FieldValue::LineId(aggr_idx));

                let mut sequence = Sequence::new();
                sequence.append_element(move_elem);
                sequence.append_element(enter_elem);
                self.launch_sequence(sequence);
                return;
            }
        }
        let _ = pc_layer;

        let mut seek_elem =
            SequenceElement::new_movement(1, Command::Seek, Some(pc_id), action_style);
        let mut enter_elem = SequenceElement::new_generic(2, Command::EnterSwordfight, Some(pc_id));
        enter_elem.set_property(Field::Opponent, FieldValue::Element(target_id));
        enter_elem.set_property(Field::JumplineDestination, FieldValue::Integer(0));
        enter_elem.command_level = 1;

        let mut post_seek = Sequence::new();
        post_seek.append_element(enter_elem);
        if let SequenceElementData::Movement {
            element,
            tolerance,
            flags,
            post_seek_sequence,
            ..
        } = &mut seek_elem.data
        {
            *element = Some(target_id);
            *tolerance = seek_tolerance;
            *flags |= MoveFlags::SEEK;
            *post_seek_sequence = Some(Box::new(post_seek));
        }

        let mut sequence = Sequence::new();
        sequence.append_element(seek_elem);
        self.launch_sequence(sequence);
    }

    fn apply_table_swordfight(
        &mut self,
        pc_id: EntityId,
        target_id: EntityId,
        aggressor_line_idx: u32,
        action_style: crate::order::OrderType,
    ) {
        let (aggressor_line, victim_line_idx) = match self
            .fast_grid
            .level
            .jump_lines
            .get(aggressor_line_idx as usize)
        {
            Some(l) => (l.clone(), l.associated_line_index),
            None => return,
        };
        let Some(victim_line) = victim_line_idx
            .and_then(|idx| self.fast_grid.level.jump_lines.get(idx as usize).cloned())
        else {
            return;
        };

        let victim_pos = match self.get_entity(target_id) {
            Some(e) => e.element_data().position_map(),
            None => return,
        };
        let t_victim = victim_line.compute_nearest_point_param(victim_pos.to_geo().into());
        let coeff = t_victim * victim_line.norm();

        let aggressor_vec = aggressor_line.vector();
        let aggressor_len = aggressor_line.norm().max(f32::EPSILON);
        let inv_len = 1.0 / aggressor_len;
        let pt_on_line = crate::coordinates::MapPoint::new(
            aggressor_line.point_b.x - coeff * aggressor_vec.x * inv_len,
            aggressor_line.point_b.y - coeff * aggressor_vec.y * inv_len,
        );

        // Plumb the line goal onto the emitted Move.  The computed
        // `pt_on_line` is already a point on the aggressor line, so
        // `MoveFlags::LINE` + `line_id` is semantic plumbing for any
        // downstream arrival check that wants to snap to line
        // tolerance.
        let mut move_elem =
            SequenceElement::new_movement(1, Command::Move, Some(pc_id), action_style);
        if let SequenceElementData::Movement {
            destination,
            tolerance,
            flags,
            line_id,
            ..
        } = &mut move_elem.data
        {
            *destination = pt_on_line;
            *tolerance = 0.0;
            *flags |= crate::sequence::MoveFlags::LINE;
            *line_id = crate::jump_line::JumpLineIndex::new(aggressor_line_idx);
        }

        let mut enter_elem = SequenceElement::new_generic(2, Command::EnterSwordfight, Some(pc_id));
        enter_elem.set_property(Field::Opponent, FieldValue::Element(target_id));
        enter_elem.set_property(
            Field::JumplineDestination,
            match crate::jump_line::JumpLineIndex::new(aggressor_line_idx) {
                Some(idx) => FieldValue::LineId(idx),
                None => FieldValue::Integer(0),
            },
        );

        let mut sequence = Sequence::new();
        sequence.append_element(move_elem);
        sequence.append_element(enter_elem);
        self.launch_sequence(sequence);
    }

    fn apply_sword_strike_with_seek(
        &mut self,
        assets: &LevelAssets,
        pc_id: EntityId,
        target_id: EntityId,
        strike_cmd: Command,
    ) {
        use crate::order::OrderType;

        let pc_sector = self
            .get_entity(pc_id)
            .and_then(|e| e.element_data().sector());
        let target_sector = self
            .get_entity(target_id)
            .and_then(|e| e.element_data().sector());
        let same_sector = matches!((pc_sector, target_sector), (Some(a), Some(b)) if a == b);

        let strike_elem = SequenceElement::new_interaction(
            if same_sector { 2 } else { 1 },
            strike_cmd,
            Some(pc_id),
            Some(target_id),
        );

        if !same_sector {
            self.launch_element(strike_elem);
            return;
        }

        let Some(strike) = (match strike_cmd {
            Command::SwordstrikeThrustA => Some(crate::weapons::SwordStrike::A),
            Command::SwordstrikeThrustB => Some(crate::weapons::SwordStrike::B),
            Command::SwordstrikeThrustC => Some(crate::weapons::SwordStrike::C),
            Command::SwordstrikeThrustD => Some(crate::weapons::SwordStrike::D),
            Command::SwordstrikeThrustE => Some(crate::weapons::SwordStrike::E),
            _ => None,
        }) else {
            tracing::warn!(
                ?pc_id,
                ?target_id,
                ?strike_cmd,
                "apply_sword_strike_with_seek: unsupported seek strike requested; launching direct strike"
            );
            self.launch_element(strike_elem);
            return;
        };

        let Some(target_distance) = self
            .get_entity(pc_id)
            .and_then(|e| crate::engine::melee::get_hth_weapon_id_full(e, &assets.profile_manager))
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .map(|p| 0.9 * p.thrusts[strike as usize].maximal_distance as f32)
        else {
            tracing::warn!(
                ?pc_id,
                ?target_id,
                ?strike_cmd,
                "apply_sword_strike_with_seek: actor has no hth weapon profile; launching direct strike"
            );
            self.launch_element(strike_elem);
            return;
        };

        let mut seek_elem = SequenceElement::new_movement(
            1,
            Command::Seek,
            Some(pc_id),
            OrderType::RunningWithSword,
        );
        let mut post_seek = Sequence::new();
        post_seek.append_element(strike_elem);
        if let SequenceElementData::Movement {
            element,
            tolerance,
            flags,
            post_seek_sequence,
            ..
        } = &mut seek_elem.data
        {
            *element = Some(target_id);
            *tolerance = target_distance;
            *flags |= MoveFlags::SEEK | MoveFlags::FORCE_SWORD_MOVEMENT;
            *post_seek_sequence = Some(Box::new(post_seek));
        }

        let mut sequence = Sequence::new();
        sequence.append_element(seek_elem);
        self.launch_sequence(sequence);
    }

    /// Build a `Seek(dest) → DropAle` compound sequence and launch
    /// it.
    ///
    fn apply_drop_ale_at(
        &mut self,
        actor: EntityId,
        target_pos: crate::coordinates::MapPoint,
        running: bool,
    ) {
        use crate::order::OrderType;

        let (posture, layer, move_box, action_distance) = match self.get_entity(actor) {
            Some(e) => {
                let action_distance = match e.sprite().action_distance(OrderType::DroppingAle) {
                    Ok(distance) => distance,
                    Err(err) => {
                        tracing::warn!(
                            ?actor,
                            error = %err,
                            "apply_drop_ale_at: missing DroppingAle action distance"
                        );
                        return;
                    }
                };
                (
                    e.element_data().posture,
                    e.element_data().layer(),
                    e.position_iface().get_move_box(),
                    action_distance,
                )
            }
            None => return,
        };

        // running → RunningUpright, else crouched → WalkingCrouched,
        // else WalkingUpright.
        let action_style = if running {
            OrderType::RunningUpright
        } else if posture == crate::element::Posture::Crouched {
            OrderType::WalkingCrouched
        } else {
            OrderType::WalkingUpright
        };

        let mut destination_pos = target_pos;
        if move_box.is_somewhere() {
            let mut box_at_target = move_box.translated(target_pos);
            if self
                .fast_grid
                .find_authorized_position(&mut box_at_target, layer)
            {
                let center = box_at_target.center();
                destination_pos = crate::coordinates::MapPoint {
                    x: center.x,
                    y: center.y,
                };
            } else {
                tracing::warn!(
                    ?actor,
                    layer,
                    target_x = target_pos.x,
                    target_y = target_pos.y,
                    "apply_drop_ale_at: target move box has no authorized position"
                );
                return;
            }
        }

        let mut move_elem =
            SequenceElement::new_movement(1, Command::Seek, Some(actor), action_style);
        if let SequenceElementData::Movement {
            destination,
            tolerance,
            flags,
            post_seek_sequence,
            ..
        } = &mut move_elem.data
        {
            *destination = destination_pos;
            *tolerance = action_distance;
            *flags |= MoveFlags::SEEK;
            let mut post_seek = Sequence::new();
            post_seek.append_element(SequenceElement::new(1, Command::DropAle, Some(actor)));
            *post_seek_sequence = Some(Box::new(post_seek));
        }

        let mut sequence = Sequence::new();
        sequence.append_element(move_elem);
        self.launch_sequence(sequence);
    }

    /// Handle the second click of the Shield two-click protocol.
    ///
    /// 1. Flip `is_protected = true` so the cursor returns to the
    ///    first-click YES/NO state.
    /// 2. Build a compound `Seek(protected_pc, tolerance=50) →
    ///    RaiseShield(Generic with ShieldDangerPoint/Layer/Protected)`
    ///    sequence and launch it.  `dispatch_raise_shield`
    ///    (`melee.rs:L1947-L1969`) reads the `ShieldDangerPoint`
    ///    property for facing.
    /// 3. Re-sync the `DangerPoint` titbit on the carrier via
    ///    `sync_danger_point_titbits`.  The titbit manager code
    ///    already sweeps `shield_danger_point` each tick, so stamping
    ///    the new value on the actor is enough.
    fn apply_raise_shield_with_danger(
        &mut self,
        actor: EntityId,
        protected_pc: EntityId,
        danger_point: crate::coordinates::MapPoint,
        danger_point_layer: u16,
    ) {
        use crate::order::OrderType;

        self.shield.is_protected = true;
        self.shield.protected_pc = Some(protected_pc);

        // Stamp the new danger point on the acting PC so
        // `sync_danger_point_titbits` refreshes the `DangerPoint`
        // titbit next tick.
        if let Some(entity) = self.entities.get_mut(actor)
            && let Some(actor_data) = entity.actor_data_mut()
        {
            actor_data.shield_face_point = Some(danger_point);
        }

        // Build Seek(protected_pc, tol=50, RUNNING_UPRIGHT) → RaiseShield.
        let mut seek_elem =
            SequenceElement::new_movement(1, Command::Seek, Some(actor), OrderType::RunningUpright);
        if let SequenceElementData::Movement {
            element,
            tolerance,
            flags,
            ..
        } = &mut seek_elem.data
        {
            *element = Some(protected_pc);
            *tolerance = 50.0;
            *flags |= crate::sequence::MoveFlags::SEEK | crate::sequence::MoveFlags::SEEK_SHIELD;
        }

        let mut raise_elem = SequenceElement::new_generic(2, Command::RaiseShield, Some(actor));
        raise_elem.set_property(
            Field::ShieldDangerPoint,
            FieldValue::Point3D {
                x: danger_point.x,
                y: danger_point.y,
                z: 0.0,
            },
        );
        raise_elem.set_property(
            Field::ShieldDangerPointLayer,
            FieldValue::Integer(u32::from(danger_point_layer)),
        );
        raise_elem.set_property(Field::ShieldProtected, FieldValue::Element(protected_pc));

        let mut post_seek = Sequence::new();
        post_seek.append_element(raise_elem);
        if let SequenceElementData::Movement {
            post_seek_sequence, ..
        } = &mut seek_elem.data
        {
            *post_seek_sequence = Some(Box::new(post_seek));
        }

        let mut sequence = Sequence::new();
        sequence.append_element(seek_elem);
        self.launch_sequence(sequence);
    }

    /// Set the protector's `shield_protected` forward pointer.
    ///
    /// Passing `protectee = None` unlinks and zeroes the protector's
    /// `shield_danger_point`; when assigning a new protectee the danger
    /// point is left untouched — the shield-raise pipeline fills it (see
    /// `dispatch_raise_shield`).
    ///
    /// Silently no-ops when the protector is not a PC; non-PC entries
    /// cannot carry the shield-protection fields.
    pub(crate) fn set_shield_protected(
        &mut self,
        protector_id: EntityId,
        protectee: Option<EntityId>,
    ) {
        if protectee.is_none()
            && let Some(me) = self.entities.get_mut(protector_id)
            && let Some(pc) = me.pc_data_mut()
        {
            pc.shield_danger_point = crate::coordinates::WorldPoint3D::default();
        }

        if let Some(me) = self.entities.get_mut(protector_id)
            && let Some(pc) = me.pc_data_mut()
        {
            pc.shield_protected = protectee;
        }
    }

    fn apply_crouch_down(&mut self, seat: usize) {
        // Route through the actor-level MakeCrouched flow so a PC
        // already walking/running gets its queued orders rewritten to
        // crouched variants instead of always launching a fresh
        // CrouchDown sequence.
        //
        // The macro step was already recorded by `record_macro_step_for`
        // at the top of dispatch; here we just skip the live posture
        // change for the currently-recording PC so the two paths don't
        // double up.  After the loop, stop the recording if a
        // recording PC was in the selection.
        let mut recorded_here = false;
        for &pc_id in &self.players.seats[seat].selection.clone() {
            if self.players.qa_recording_for.contains(&pc_id) {
                recorded_here = true;
                continue;
            }
            self.actor_make_crouched(pc_id);
        }
        if recorded_here {
            self.stop_recording_macro();
        }
    }

    fn apply_stand_up(&mut self, seat: usize) {
        let mut recorded_here = false;
        for &pc_id in &self.players.seats[seat].selection.clone() {
            if self.players.qa_recording_for.contains(&pc_id) {
                recorded_here = true;
                continue;
            }
            let posture = self
                .get_entity(pc_id)
                .map(|e| e.element_data().posture)
                .unwrap_or(crate::element::Posture::Upright);

            match posture {
                crate::element::Posture::Crouched => {
                    // Try rewriting the active movement sequence
                    // first, falling back to a fresh CrouchUp launch
                    // only when no active sequence is present.
                    self.actor_make_upright(pc_id);
                }
                crate::element::Posture::SimulatingBeggar => {
                    let elem = SequenceElement::new(1, Command::LeaveBeggar, Some(pc_id));
                    self.launch_element(elem);
                }
                crate::element::Posture::Spy | crate::element::Posture::AnonymousArcher => {
                    let elem = SequenceElement::new(1, Command::LeaveSpy, Some(pc_id));
                    self.launch_element(elem);
                }
                crate::element::Posture::Tree => {
                    let elem = SequenceElement::new(1, Command::LeaveTree, Some(pc_id));
                    self.launch_element(elem);
                }
                _ => continue,
            };
        }
        if recorded_here {
            self.stop_recording_macro();
        }
    }

    fn apply_box_select(
        &mut self,
        assets: &LevelAssets,
        input: &mut InputState,
        seat: usize,
        pt1: crate::coordinates::MapPoint,
        pt2: crate::coordinates::MapPoint,
        shift: bool,
    ) {
        input.multi_selection_pt1 = pt1;
        input.multi_selection_pt2 = pt2;
        input.draw_multi_selection = true;
        input.multi_selection_active = true;
        self.perform_multi_selection(assets, input, seat, shift);
    }

    fn apply_box_unselect(
        &mut self,
        input: &mut InputState,
        seat: usize,
        pt1: crate::coordinates::MapPoint,
        pt2: crate::coordinates::MapPoint,
    ) {
        input.multi_selection_pt1 = pt1;
        input.multi_selection_pt2 = pt2;
        input.draw_multi_selection = true;
        input.multi_unselection_active = true;
        self.perform_multi_unselection(input, seat);
    }
}

/// Rewrite a focused object id so coin clicks target the whole
/// source purse when that purse is still standing.
///
/// Before launching the Take sequence, if the coin was ejected from
/// a not-yet-taken purse, route the click to the purse — the
/// follow-up pickup then runs the purse take handler on arrival and
/// sweeps every still-active sibling coin in one call via
/// [`EngineInner::take_purse`].  Loose coins (no source purse) and
/// coins whose source purse has already been taken pass through
/// unchanged and go through the base coin pickup.
pub fn coin_pickup_target(engine: &EngineInner, target_id: EntityId) -> EntityId {
    let Some(crate::element::Entity::Projectile(p)) = engine.get_entity(target_id) else {
        return target_id;
    };
    if p.object.object_type != crate::element::ObjectType::Coin {
        return target_id;
    }
    let Some(purse_id) = p.projectile.purse.source_purse else {
        return target_id;
    };
    // Purse missing / already-taken → stay on the coin.
    match engine.get_entity(purse_id) {
        Some(crate::element::Entity::Projectile(purse))
            if matches!(
                purse.object.object_type,
                crate::element::ObjectType::Purse | crate::element::ObjectType::BonusPurse
            ) && !purse.object.taken
                && purse.element.active =>
        {
            purse_id
        }
        _ => target_id,
    }
}

/// Object pickup gate.
///
/// Returns `true` when the given PC can pick up the given object right
/// now:
///
/// * `associated_action == NoAction` (scrolls, relics, amulets, ransom
///   bags, coins — anything that doesn't fill an ammo slot) → always
///   takable.
/// * PC has the associated action AND has at least one free ammo slot
///   (max ammo − current > 0) → takable.
/// * Fallback for Eat bonuses: when the PC has Guzzle instead, the
///   bonus still picks up if the guzzle slot has room.
pub(super) fn is_pc_takable(
    engine: &EngineInner,
    assets: &LevelAssets,
    object: &crate::element::Entity,
    pc_id: EntityId,
) -> bool {
    use crate::profiles::Action;

    let Some(obj) = object.object_data() else {
        return false;
    };
    // Amulet max-count gate — refuse any further amulet pickups
    // once the campaign's Amulets counter reaches the maximum.
    // Runs before the `NoAction → true` fast-path because amulets
    // themselves carry `Action::NoAction`.
    if obj.object_type == crate::element::ObjectType::BonusAmulet
        && let Some(campaign) = engine.mission_domain.campaign.as_ref()
        && campaign.get_value(crate::campaign::CampaignValue::Amulets)
            >= crate::campaign::MAXIMUM_AMULETS_NUMBER
    {
        return false;
    }
    let assoc = obj.associated_action;
    if assoc == Action::NoAction {
        return true;
    }
    let Some(pc) = engine.get_entity(pc_id) else {
        return false;
    };
    let Some(pc_data) = pc.pc_data() else {
        return false;
    };
    let Some(profile) = assets
        .profile_manager
        .characters
        .get(usize::from(pc_data.profile_index))
    else {
        return false;
    };

    let difficulty = crate::player_profile::DifficultyLevel::current();

    // Resolve PC status to read current ammo.  Pulled lazily because
    // not every branch needs it (NoAction returns early above).
    let Some(pc_desc) = engine.pc_description_for_pc_data(pc_data) else {
        return false;
    };
    let status = &pc_desc.status;

    let storage_left_for = |action: Action| -> u16 {
        let max = crate::inventory::max_ammo_for_action(profile, action, difficulty);
        let current = status.get_ammo(action);
        max.saturating_sub(current)
    };

    // `find_action_slot` already folds Eat→Guzzle, so the explicit
    // Guzzle fallback is unnecessary here.
    if crate::inventory::find_action_slot(profile, assoc).is_some() {
        return storage_left_for(assoc) > 0;
    }
    false
}

/// Click-to-pickup dispatch for an object-class entity.
///
/// Per-subclass behaviour:
///
/// * Net — landed nets, always takable (the net is never stored as
///   ammo directly; the action check lives upstream in
///   `is_object_focusable`).
/// * Coin — forwards to the source purse when the purse hasn't been
///   taken yet; when the purse is still live
///   [`coin_pickup_target`] rewrites the target to the purse id, so
///   the Take sequence lands on the whole purse instead of one coin.
/// * Bonus / Scroll / landed Projectile — base path: `Seek` to object,
///   `Take` on arrival.
///
/// Upstream focus checks (`engine::input::is_object_focusable`)
/// already gated everything we care about, so this helper is narrow:
/// apply the per-type takability gate and translate the focused
/// object into the `Take` command the caller feeds to
/// `apply_interaction_with_seek`.
///
/// Returns `None` when the entity isn't a pickup-style object, or
/// when the object isn't currently in a takable state (e.g. an
/// Invisible scroll, a flying projectile, a taken bonus, or a bonus
/// whose PC already has a full inventory slot for its action).
pub fn object_pickup_command(
    engine: &EngineInner,
    assets: &LevelAssets,
    target_id: EntityId,
    pc_id: EntityId,
) -> Option<Command> {
    use crate::element::{Entity, ObjectType};

    let entity = engine.get_entity(target_id)?;

    // Macro-record escape hatch: when recording AND the PC owns the
    // object's associated action, bypass the full-inventory takable
    // gate so the step gets captured into the macro (the replay will
    // re-check takability at firing time).  The cursor path mirrors
    // this in `engine::input::choose_object_cursor`.
    let macro_override = || -> bool {
        if !engine.is_recording_macro() {
            return false;
        }
        let Some(obj) = entity.object_data() else {
            return false;
        };
        let Some(pc) = engine.get_entity(pc_id).and_then(|e| e.pc_data()) else {
            return false;
        };
        assets
            .profile_manager
            .get_character(pc.profile_index)
            .is_some_and(|profile| profile.has_action(obj.associated_action))
    };

    match entity {
        // Net: skips the takable gate entirely; the action-ownership
        // check is handled upstream in `is_object_focusable`.
        Entity::Net(n) if !n.projectile.flying => Some(Command::Take),

        // Bonus items: route through `is_pc_takable` — a full
        // inventory slot means the click is a no-op unless the
        // macro-record escape hatch fires.
        Entity::Bonus(b) => (b.is_takable()
            && (is_pc_takable(engine, assets, entity, pc_id) || macro_override()))
        .then_some(Command::Take),

        // Scrolls: no associated action; takable is vacuously true
        // once status is Visible / Opened.
        Entity::Scroll(_) => {
            use super::scroll_reveal::ScrollStatus;
            matches!(
                engine.scroll_status(target_id),
                ScrollStatus::Visible | ScrollStatus::Opened
            )
            .then_some(Command::Take)
        }

        // Projectile (landed coin/purse/stone/arrow/etc.): per-type
        // filter (Apple/WaspNest/Wasp never focusable) + `is_pc_takable`
        // (with the same macro-record escape hatch).
        Entity::Projectile(p) if !p.projectile.flying && !p.object.taken => {
            match p.object.object_type {
                ObjectType::Apple
                | ObjectType::BonusApple
                | ObjectType::WaspNest
                | ObjectType::BonusWaspNest
                | ObjectType::Wasp => None,
                _ => (is_pc_takable(engine, assets, entity, pc_id) || macro_override())
                    .then_some(Command::Take),
            }
        }
        _ => None,
    }
}

/// Determine which Use command to launch on a target entity.
/// Public so apply_enter_swordfight can call it.
fn determine_use_command(
    engine: &EngineInner,
    assets: &LevelAssets,
    pc_id: EntityId,
    target_id: EntityId,
) -> Option<Command> {
    let entity = engine.get_entity(target_id)?;

    // FX targets — walk the target's `GetCommand` filter ladder.
    // Search / Lever / Money are gated on the PC's contextual
    // abilities and VIP flag.
    if let crate::element::Entity::Target(t) = entity {
        let pc_char_profile = engine
            .get_entity(pc_id)
            .and_then(|e| e.pc_data())
            .and_then(|pc| assets.profile_manager.get_character(pc.profile_index));
        let pc_has_search =
            pc_char_profile.is_some_and(|p| p.has_contextual_action(Action::Search));
        let pc_has_lever = pc_char_profile.is_some_and(|p| p.has_contextual_action(Action::Lever));
        let pc_is_vip = engine
            .get_entity(pc_id)
            .is_some_and(|e| engine.is_entity_vip(assets, e));
        return super::target_interaction::target_use_command(
            t.target.action_filter,
            pc_has_search,
            pc_has_lever,
            pc_is_vip,
        );
    }

    // Object-class targets (Net, Bonus, Scroll, landed Projectile)
    // route through the shared per-type dispatch.
    if let Some(cmd) = object_pickup_command(engine, assets, target_id, pc_id) {
        return Some(cmd);
    }

    // Scroll / Bonus / landed Projectile pickup.
    // `is_object_focusable(Focus::Use)` already gated status / focus.
    if let crate::element::Entity::Scroll(_) = entity {
        return Some(Command::Take);
    }
    if let crate::element::Entity::Bonus(_) = entity {
        return Some(Command::Take);
    }
    if let crate::element::Entity::Projectile(p) = entity
        && !p.projectile.flying
    {
        return Some(Command::Take);
    }

    let is_dead = entity.is_dead();
    let posture = entity.element_data().posture;
    let is_unconscious = entity.human_data().is_some_and(|h| h.unconscious);
    let is_tied = posture == crate::element::Posture::Tied;

    // PC override fires before the human fallback.  When the target
    // PC is in HelpingToClimb posture and the selector PC has Jump,
    // dispatch the climb-up-on-shoulders sequence.
    // `is_entity_focusable(Focus::Use)` already gates on
    // `posture == HelpingToClimb && has_jump && !selector_swordfighting`
    // (engine/input.rs:508-524).
    if matches!(entity, crate::element::Entity::Pc(_))
        && posture == crate::element::Posture::HelpingToClimb
    {
        if engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            crate::profiles::Action::Jump,
        ) {
            return Some(Command::ClimbUpOnShoulders);
        }
        return None;
    }

    // Pay beggar — alive, conscious beggar civilian whose VIP
    // selector has enough ransom.  Silently no-op when
    // ransom < BEGGAR_SALARY, even though the focus and cursor still
    // light up (PayNo).  The ransom check therefore lives here, not
    // in `is_entity_focusable`.
    if !is_dead
        && !is_unconscious
        && posture != crate::element::Posture::Carried
        && matches!(entity, crate::element::Entity::Civilian(c)
            if c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar
                && !c.npc.scroll_attached)
    {
        let ransom = engine
            .mission_domain
            .campaign
            .as_ref()
            .map(|c| c.get_value(crate::campaign::CampaignValue::Ransom))
            .unwrap_or(0);
        if ransom >= crate::engine::BEGGAR_SALARY {
            return Some(Command::Pay);
        }
        return None;
    }

    if is_dead {
        return Some(Command::SearchCmd);
    }
    if !is_dead && !is_unconscious && posture == crate::element::Posture::Lying {
        return Some(Command::SearchCmd);
    }

    // Wake-Up arm: `(IsPc || (IsSoldier && same camp as selected PC))
    // && is_unconscious && selector has Resuscitate`.  Selected PCs
    // are always Royalists, so the camp test reduces to
    // `Soldier::cached_camp == Royalists`.
    if is_unconscious
        && engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            crate::profiles::Action::Resuscitate,
        )
    {
        let target_pc_or_same_camp = match entity {
            crate::element::Entity::Pc(_) => true,
            crate::element::Entity::Soldier(s) => {
                s.soldier.cached_camp == crate::element::Camp::Royalists
            }
            _ => false,
        };
        if target_pc_or_same_camp {
            return Some(Command::WakeUp);
        }
    }

    // Take-Corpse arm: `(is_dead || is_unconscious) &&
    // (LittleJohnCarry || FarmerCarry) && !is_heavy`.  Ordered before
    // Tie so an unconscious soldier the PC can carry doesn't get
    // mis-routed to Tie when the PC lacks the Tie ability.
    if (is_unconscious || is_dead)
        && posture != crate::element::Posture::Carried
        && !is_tied
        && engine.selected_pc_can_carry(assets, Some(pc_id))
    {
        let is_heavy = match entity {
            crate::element::Entity::Soldier(s) => assets
                .profile_manager
                .get_soldier(s.soldier.soldier_profile_index)
                .map(|p| p.heavy)
                .unwrap_or(false),
            _ => false,
        };
        if !is_heavy {
            return Some(Command::TakeCorpse);
        }
    }

    // Tie arm: `is_unconscious && posture == Lying && selector has Tie`.
    // Without the carry path or the Tie ability, the click no-ops.
    if is_unconscious
        && !is_tied
        && posture != crate::element::Posture::Carried
        && engine.selected_pc_has_contextual_action(
            assets,
            Some(pc_id),
            crate::profiles::Action::Tie,
        )
    {
        return Some(Command::TieCmd);
    }
    None
}

/// Per-object Take seek tolerance = `radius + 15`.
///
/// Per-subclass radius:
///   * Ale → 5 (tolerance 20)
///   * Purse → 7 (tolerance 22)
///   * Coin → 3 (tolerance 18)
///   * Net → 40 uncrumpled / 10 crumpled (55 / 25)
///   * Everything else (plain bonus / scroll / arrow / stone / cape /
///     apple / wasp / waspnest) → 0 (tolerance 15).
fn take_seek_tolerance(entity: &crate::element::Entity) -> f32 {
    use crate::element::{Entity, ObjectType};
    let radius: f32 = match entity {
        Entity::Bonus(b) => match b.object.object_type {
            ObjectType::Ale => 5.0,
            ObjectType::Purse => 7.0,
            _ => 0.0,
        },
        Entity::Projectile(p) => match p.object.object_type {
            ObjectType::Ale => 5.0,
            ObjectType::Purse => 7.0,
            ObjectType::Coin => 3.0,
            _ => 0.0,
        },
        Entity::Net(n) => {
            if n.net.crumpled {
                10.0
            } else {
                40.0
            }
        }
        _ => 0.0,
    };
    radius + 15.0
}

/// Animation whose sprite-script action distance drives a
/// seek-before-interact command.
pub(crate) fn command_action_distance_animation(cmd: Command) -> Option<crate::order::OrderType> {
    use crate::order::OrderType;

    match cmd {
        Command::StrangleCmd => Some(OrderType::Strangling),
        Command::HealCmd => Some(OrderType::Healing),
        Command::TieCmd => Some(OrderType::Tying),
        Command::TakeCorpse => Some(OrderType::TransitionWaitingUprightCarryingCorpse),
        Command::ClimbUpOnShoulders => Some(OrderType::ClimbingUpOnShoulders),
        Command::SearchCmd => Some(OrderType::Searching),
        Command::HitCmd => Some(OrderType::Hitting),
        Command::RaiseShield => Some(OrderType::RaisingShield),
        Command::Pay => Some(OrderType::Paying),
        Command::WakeUp => Some(OrderType::WakingUp),
        Command::UseLever => Some(OrderType::UsingLever),
        _ => None,
    }
}

/// Default action distances for interactions that use
/// seek-before-interact but do not have a known sprite-script action
/// distance mapping in the original engine.
///
/// `Command::Take` deliberately omitted: the per-object `radius + 15`
/// lookup lives in `take_seek_tolerance` and is consulted at the call
/// site in `apply_interaction_with_seek`.
pub(crate) fn interaction_distance(cmd: Command) -> f32 {
    match cmd {
        Command::StrangleCmd => 30.0,
        Command::HealCmd => 35.0,
        Command::TieCmd => 25.0,
        Command::TakeCorpse => 25.0,
        Command::ClimbUpOnShoulders => 8.0,
        Command::SearchCmd => 25.0,
        Command::ShootBow => 0.0, // bow has no walk-up
        Command::HitCmd => 30.0,
        Command::ThrowApple | Command::ThrowStone => 0.0, // ranged
        Command::RaiseShield => 35.0,
        // `Command::Take` is normally handled by `take_seek_tolerance`;
        // this arm is a defensive fallback (Ale-radius 5 + 15 = 20)
        // for call paths that resolve Take without an entity in hand.
        Command::Take => 20.0,
        // Pay uses 0 — the VIP walks right up to the beggar.
        Command::Pay => 0.0,
        _ => 30.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        ActorCivilian, ActorData, ActorPc, ElementBonus, ElementData, ElementKind, ElementNet,
        ElementProjectile, ElementScroll, Entity, HumanData, NetData, NpcData, ObjectData,
        ObjectType, PcData, Posture, ProjectileData,
    };
    use crate::engine::MissionScript;
    use crate::engine::ScrollStatus;
    use crate::macro_store::{QaReplayCommand, QuickActionStep};
    use crate::profiles::{Action, CharacterProfile, ProfileManager};
    use crate::sprite::Sprite;
    use crate::sprite_script::{SpriteScript, UNMAPPED};

    /// Build an `(engine, assets, pc_id)` triple with a single PC
    /// whose character profile carries the supplied `(action, max_ammo)`
    /// pairs.  The PC's live ammo counts start at 0 — so storage-left
    /// equals `max_ammo` for every configured action, which is what
    /// the IsTakable tests want.
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
        engine.mission_domain.campaign = Some(campaign);

        let pc_id = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                profile_index: crate::profiles::CharacterProfileIdx(0),
                life_points: 50,
                ..PcData::default()
            },
        }));

        (engine, assets, pc_id)
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
        let status_idx = 1u8;

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
        campaign.characters.push(crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            instanced: false,
            ..Default::default()
        });
        campaign.characters.push(crate::campaign::PcDescription {
            character_profile_idx: Some(profile_idx),
            instanced: true,
            ..Default::default()
        });
        engine.mission_domain.campaign = Some(campaign);

        let pc_id = engine.add_entity(Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                profile_index: profile_idx,
                list_index: status_idx,
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
            element: ElementData {
                kind: ElementKind::ObjectBonus,
                active,
                ..Default::default()
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
            element: ElementData {
                kind: ElementKind::ObjectScroll,
                active,
                ..Default::default()
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
            element: ElementData {
                kind: ElementKind::ObjectProjectile,
                active: true,
                ..Default::default()
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
        let mut element = ElementData {
            kind: ElementKind::ObjectNet,
            active: true,
            ..Default::default()
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
        element.sprite = sprite;
        element.set_position_map(position);
        element.set_direction_instantly(direction);
    }

    fn spawn_pc_at(engine: &mut EngineInner, x: f32, y: f32) -> EntityId {
        let mut pc = ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        };
        pc.element
            .set_position_map(crate::coordinates::MapPoint { x, y });
        engine.add_entity(Entity::Pc(pc))
    }

    fn first_seek_tolerance(engine: &EngineInner) -> f32 {
        let sequence = engine.sequence_manager.sequences_iter().next().unwrap();
        let seek = sequence.get(0).unwrap();
        match &seek.data {
            SequenceElementData::Movement { tolerance, .. } => *tolerance,
            other => panic!("expected movement seek element, got {other:?}"),
        }
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
        engine.mission_script = Some(minimal_script());
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
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                scroll_attached: true,
                ..NpcData::default()
            },
            civilian: Default::default(),
        };
        npc.element
            .set_position_map(crate::coordinates::MapPoint { x: 110.0, y: 100.0 });
        let npc_id = engine.add_entity(Entity::Civilian(npc));

        let scroll_id = spawn_scroll(&mut engine, true);
        engine
            .mission_script
            .as_mut()
            .unwrap()
            .game_host
            .scroll_attachments
            .insert(
                crate::natives::GameHost::actor_handle(npc_id),
                crate::natives::GameHost::actor_handle(scroll_id),
            );

        (engine, assets, pc_id, npc_id, scroll_id)
    }

    fn assert_scroll_read_composite(
        sequence: &Sequence,
        pc_id: EntityId,
        npc_id: EntityId,
        scroll_id: EntityId,
    ) {
        assert_eq!(sequence.len(), 5);
        assert_eq!(sequence.get(0).unwrap().command, Command::LockAi);
        assert_eq!(sequence.get(0).unwrap().owner, Some(npc_id));
        assert_eq!(sequence.get(1).unwrap().command, Command::TurnElement);
        assert_eq!(sequence.get(1).unwrap().owner, Some(pc_id));
        assert_eq!(sequence.get(2).unwrap().command, Command::TurnElement);
        assert_eq!(sequence.get(2).unwrap().owner, Some(npc_id));
        assert_eq!(sequence.get(3).unwrap().command, Command::UnlockAi);
        assert_eq!(sequence.get(3).unwrap().owner, Some(npc_id));

        let open = sequence.get(4).unwrap();
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
        let (mut engine, assets, pc_id, npc_id, _scroll_id) = setup_scroll_read_scene();
        let mut display = HostDisplayState::default();
        let mut input = InputState::default();

        engine.apply_command(
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartRecordingMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );
        engine.apply_command(
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::LaunchScrollRead {
                actor: pc_id,
                target: npc_id,
                running: false,
            },
        );

        assert_eq!(engine.sequence_manager.sequence_count(), 0);
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
    }

    #[test]
    fn scroll_read_macro_replay_rebuilds_live_sequence_shape() {
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
            &mut display,
            &mut input,
            &assets,
            &PlayerCommand::StartMacro {
                pc: Some(pc_id),
                slot: 0,
            },
        );

        assert_eq!(engine.sequence_manager.sequence_count(), 1);
        let sequence = engine.sequence_manager.sequences_iter().next().unwrap();
        assert_scroll_read_composite(sequence, pc_id, npc_id, scroll_id);
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
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Lying,
                ..ElementData::default()
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
        );

        let sequence = engine.sequence_manager.sequences_iter().next().unwrap();
        let seek = sequence.get(0).unwrap();
        match &seek.data {
            SequenceElementData::Movement { tolerance, .. } => {
                assert!((*tolerance - 13.0).abs() < 0.001);
            }
            other => panic!("expected movement seek element, got {other:?}"),
        }
    }

    #[test]
    fn mapped_interaction_seek_tolerance_uses_sprite_action_distance() {
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
            crate::coordinates::SpriteLocalPoint::new(19.0, 0.0),
            crate::coordinates::SpriteAnchor::new(0.0, 0.0),
        );
        let target_id = spawn_pc_at(&mut engine, 90.0, 10.0);

        engine.apply_interaction_with_seek(pc_id, target_id, Command::SearchCmd, false);

        assert!((first_seek_tolerance(&engine) - 19.0).abs() < 0.001);
    }

    #[test]
    fn shoot_bow_interaction_launches_without_seek() {
        let (mut engine, _assets, pc_id) = setup_pc_engine(&[(Action::Bow, 1)]);
        {
            let pc = engine.get_entity_mut(pc_id).unwrap().element_data_mut();
            pc.set_position_map(crate::coordinates::MapPoint { x: 10.0, y: 10.0 });
        }
        let target_id = spawn_pc_at(&mut engine, 90.0, 10.0);

        engine.apply_interaction_with_seek(pc_id, target_id, Command::ShootBow, false);

        assert_eq!(engine.sequence_manager.sequence_count(), 1);
        let sequence = engine.sequence_manager.sequences_iter().next().unwrap();
        let element = sequence.get(0).unwrap();
        assert_eq!(element.command, Command::ShootBow);
        assert!(matches!(
            element.data,
            SequenceElementData::Interaction { .. }
        ));
    }

    #[test]
    fn mapped_interaction_missing_sprite_action_distance_noops() {
        let (mut engine, _assets, pc_id) = setup_pc_engine(&[(Action::Hit, 0)]);
        let target_id = spawn_pc_at(&mut engine, 90.0, 10.0);

        engine.apply_interaction_with_seek(pc_id, target_id, Command::HitCmd, false);

        assert!(engine.sequence_manager.sequences_iter().next().is_none());
    }

    #[test]
    fn climb_on_shoulders_seek_tolerance_uses_sprite_action_distance() {
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

        assert!((first_seek_tolerance(&engine) - 11.0).abs() < 0.001);
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
    fn pickup_dispatch_uses_pc_status_index_not_profile_index() {
        // C++ PCs read ammo from their own mpStatus, not from a
        // campaign array slot equal to the static profile index.
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
        // portrait slot 0. C++ GetActionIndex(action) disables the
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
        if let Some(campaign) = engine.mission_domain.campaign.as_mut()
            && let Some(pc_desc) = campaign.characters.get_mut(0)
        {
            pc_desc.status.set_ammo(Action::Heal, 3);
        }
        let id = spawn_bonus(&mut engine, ObjectType::BonusPlants, true, Action::Heal);
        assert_eq!(object_pickup_command(&engine, &assets, id, pc_id), None);
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
        // Original RHElementBonus::IsTakable delegates relics to the
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
            element: ElementData {
                kind: ElementKind::ObjectProjectile,
                active: true,
                ..Default::default()
            },
            object: ObjectData {
                object_type: ObjectType::Purse,
                ..Default::default()
            },
            projectile: ProjectileData::default(),
        }));
        let coin_id = engine.add_entity(Entity::Projectile(ElementProjectile {
            element: ElementData {
                kind: ElementKind::ObjectProjectile,
                active: true,
                ..Default::default()
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
            element: ElementData {
                kind: ElementKind::ObjectProjectile,
                active: true,
                ..Default::default()
            },
            object: ObjectData {
                object_type: ObjectType::Purse,
                taken: true,
                ..Default::default()
            },
            projectile: ProjectileData::default(),
        }));
        let coin_id = engine.add_entity(Entity::Projectile(ElementProjectile {
            element: ElementData {
                kind: ElementKind::ObjectProjectile,
                active: true,
                ..Default::default()
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
    fn connect_seat_creates_and_names_peer() {
        use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
        let (mut engine, assets, _pc_id) = setup_pc_engine(&[]);
        let mut input = InputState::default();
        let mut display = HostDisplayState::default();

        // Host issues a ConnectSeat for peer 2.  The dispatch `seat`
        // is HOST (0) but the command's payload targets PlayerId(2).
        engine.apply_commands(
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
    fn disconnect_then_reconnect_preserves_selection() {
        use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
        let (mut engine, assets, _pc_id) = setup_pc_engine(&[]);
        let mut input = InputState::default();
        let mut display = HostDisplayState::default();

        // Connect seat 2, give it a fake selection, disconnect, reconnect.
        engine.apply_commands(
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
        use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
        let (mut engine, assets, _pc_id) = setup_pc_engine(&[]);
        let mut input = InputState::default();
        let mut display = HostDisplayState::default();

        // Bring up peer 2 then have it toggle alt-lock — host seat
        // must be unaffected.
        engine.apply_commands(
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
        use crate::player_command::{PlayerCommand, PlayerId, PlayerInput};
        let (mut engine, assets, _pc_id) = setup_pc_engine(&[]);
        let mut input = InputState::default();
        let mut display = HostDisplayState::default();

        engine.apply_commands(
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
}
