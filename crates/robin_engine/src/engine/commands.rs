//! Player command dispatch — applies [`PlayerCommand`]s to the engine.
//!
//! This is the single entry point for all player-initiated sim mutations.
//! The input system resolves raw events into commands by reading engine
//! state immutably; this module executes them.

use super::movement::GoalShape;
use super::{EngineInner, HostDisplayState, InputState, LevelAssets};
use crate::coordinates::MapPoint;
use crate::element::{Command, Entity, EntityId, Human as _};
use crate::player_command::{PlayerCommand, PlayerInput};
use crate::profiles::Action;
use crate::sequence::{
    Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
};
use crate::titbit::{ElementHandle, INVALID_ID, QuickAction, TitbitKind};

#[derive(Clone, Copy, PartialEq, Eq)]
enum QuickActionRecordingStore {
    Manual,
    Automatic,
}

#[inline]
fn group_move_actor_accepts_command(actor: EntityId, recorded_failed_routes: &[EntityId]) -> bool {
    !recorded_failed_routes.contains(&actor)
}

/// Rebuild the seek tolerance of a `SwordStrikeCmd` that was recorded
/// before the resolved distance became part of the command.
///
/// `RHEngine::PerformSwordfight` derives the tolerance from the *mouse
/// pattern*, not from the launched command:
/// `0.9f * GetStrikeMaximalDistance( ConvertMousePatternToStrike( mousePattern ) )`
/// (`original-code/RHengine.cpp:15799` for the no-gesture click arm,
/// `original-code/RHengine.cpp:15846` for the recognised A–E gesture arm).
///
/// The two arms disagree because
/// `RHEngine::ConvertMousePatternToStrike` returns `END_OF_REAL_STRIKE`
/// for every unrecognised pattern (`original-code/RHengine.cpp:15910-15942`),
/// and `RHSword::GetStrikeMaximalDistance` answers that with the
/// weapon's generic `auwDistance[ MAXIMAL ]` instead of the per-thrust
/// `athrust[ strike ].uwMaximalDistance`
/// (`original-code/RHSword.cpp:245-251`).
///
/// A legacy record carries the command but not the pattern, so the
/// pattern is recovered from the command:
/// * The click arm hard-codes `RHCOMMAND_SWORDSTRIKE_THRUST_A` while its
///   pattern stays `MOUSEWAYPATTERN_NONE`
///   (`original-code/RHengine.cpp:15801-15804`), so a legacy thrust-A
///   seek is reconstructed with the generic maximum.
/// * `RHCOMMAND_SWORDSTRIKE_THRUST_B..E` are only ever launched by the
///   gesture arm, whose pattern maps 1:1 onto `SWORDSTRIKE_B..E`, so
///   those keep the per-thrust maximum.
fn legacy_sword_seek_distance(
    weapon: &crate::profiles::HtHWeaponProfile,
    strike_cmd: Command,
    strike: crate::weapons::SwordStrike,
) -> f32 {
    let maximum = if strike_cmd == Command::SwordstrikeThrustA {
        weapon.distance[crate::weapons::WeaponDistance::Maximal as usize]
    } else {
        weapon.thrusts[strike as usize].maximal_distance
    };
    0.9 * maximum as f32
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordedInteractionIdentityError {
    MissingOrNonPcActor,
    MissingTarget,
}

/// Preserve `AppendMoveToSequence`'s sector test after adapting an actor
/// already committed to a door onto that door's far side.
///
/// Original performs the adaptation before `pSectorGoal != pSectorSource`
/// (`original-code/RHsequence.cpp`). If the far side is already the target
/// sector, the route is a direct Move and must not gain a leading
/// AssertPosition merely because the actor's raw sector differed earlier.
fn target_interaction_assert_source_sector(
    adapted_source_sector: crate::position_interface::SectorHandle,
    target_sector: crate::position_interface::SectorHandle,
) -> Option<crate::position_interface::SectorHandle> {
    let same_sector = match (
        adapted_source_sector.arena_index(),
        target_sector.arena_index(),
    ) {
        (Some(source), Some(target)) => source == target,
        (None, None) => adapted_source_sector == target_sector,
        (Some(_), None) | (None, Some(_)) => false,
    };
    (!same_sector).then_some(adapted_source_sector)
}

impl EngineInner {
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
    pub fn apply_commands(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        commands: &[PlayerInput],
    ) {
        for (index, inp) in commands.iter().enumerate() {
            let seat = self.ensure_seat(inp.player_id);
            // Original RHMessenger::ForwardMessage is recursive. The parity
            // recorder therefore retains both the root SelectPC message and
            // its depth-2 SelectAction restitution as adjacent commands (see
            // RHParity::ShouldRecordNestedSelection). During replay the
            // recorded child is authoritative; synthesizing it again inside
            // SelectPC can launch work from stale hidden action state before
            // the recorded child corrects that state.
            let recorded_nested_selection_action = matches!(
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
                display,
                input,
                assets,
                seat,
                &inp.command,
                recorded_nested_selection_action,
            );
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
        let sim = self.control.simulation_context();
        let sim = &sim;
        for cmd in commands {
            self.apply_command(sim, display, input, assets, cmd);
        }
    }

    /// Apply a single [`PlayerCommand`] as if it came from
    /// [`crate::player_command::PlayerId::HOST`].
    ///
    /// Thin wrapper around [`Self::apply_command_for_seat`] used by
    /// the single-player input path and by tests.
    pub fn apply_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        cmd: &PlayerCommand,
    ) {
        self.apply_command_for_seat(sim, display, input, assets, 0, cmd);
    }

    /// Apply a single player command issued by `seat`.
    ///
    /// `seat` is the index returned by [`Self::ensure_seat`].
    /// Selection-mutating handlers index `self.players.seats[seat]` so
    /// different players don't clobber each other's selections.
    pub fn apply_command_for_seat(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        seat: usize,
        cmd: &PlayerCommand,
    ) {
        self.apply_command_for_seat_with_replay_context(
            sim, display, input, assets, seat, cmd, false,
        );
    }

    fn apply_command_for_seat_with_replay_context(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        seat: usize,
        cmd: &PlayerCommand,
        recorded_nested_selection_action: bool,
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

        // Append-while-recording hook.  Records one `QuickActionStep`
        // per sim-affecting player command addressed at the currently
        // recording PC, keyed by the resolved Action (portrait bar)
        // so the macro-icon strip can render per-step titbit frames.
        self.record_macro_step_for(seat, cmd, assets);
        match cmd {
            Noop => {} // consumed input, no action

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
                let live_actors: Vec<_> = actors
                    .iter()
                    .copied()
                    .filter(|actor| !self.players.qa_recording_for.contains(actor))
                    .collect();
                if live_actors.len() != actors.len() {
                    // The shared recorder captured each armed PC above. A
                    // manually recorded move must not also move that live PC.
                    self.stop_recording_macro();
                }
                if live_actors.is_empty() {
                    return;
                }
                self.perform_group_move(
                    sim,
                    assets,
                    &live_actors,
                    *destination,
                    *running,
                    *show_marker,
                    *goal_override,
                    *goal_sector_index_override,
                    *door_route_override,
                    recorded_gate_routes,
                    recorded_failed_gate_routes,
                );
                // Fire `HeroSpeaking(HERO_ACCEPT_COMMAND, 0)` for the PC
                // that just accepted the move — the "yes, milord" bark.
                // It lives outside `perform_group_move` because the engine
                // helper has no access to `LevelAssets`; this is the
                // command-dispatch entry point where the assets are in
                // scope.
                for &pc_id in &live_actors {
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
                // RHElementActor::Stop leaves the actor's default Wait
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
                let recording_interaction = self.players.qa_recording_for.contains(actor);
                // Macro recording: if `actor` is in the recording set
                // and a slot is armed, append this interaction as a step.
                if recording_interaction {
                    let (pos, tgt_layer, tgt_is_pc, tgt_is_object, tgt_target_filter) = self
                        .get_entity(*target)
                        .map(|e| {
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
                        .expect("recorded interaction target passed strict preflight");
                    let action = self
                        .get_entity(*actor)
                        .and_then(|e| e.pc_data())
                        .map(|pc| pc.current_action)
                        .expect("recorded interaction PC passed strict preflight");
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
                        *running,
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
                    //
                    // TODO(parity): RHParity::RecordInteraction merges
                    // command-specific Original recording sites. Audit their
                    // authored RHQUICK phases, including direct ShootBow and
                    // TakeCorpse, instead of deriving every phase from the
                    // actor's current Action here.
                }
                if recording_interaction {
                    // AddInteractionWithSeek stores the constructed sequence
                    // in the active QA slot and sends STOP_RECORDING_MACRO;
                    // it does not also launch that sequence live. The parity
                    // trace records the semantic interaction before this
                    // branch, so replay must preserve the recording-only
                    // disposition explicitly.
                    self.stop_recording_macro();
                    return;
                }
                // Schema-9 records every resolved interaction under one
                // command shape, although the Original has several route
                // constructors. Commands resolved against RHElementTarget
                // use RHElementTarget::MouseClicked's ordinary click path,
                // which directly builds AppendMoveToSequence rather than
                // calling AddInteractionWithSeek. Keep the command gate
                // aligned with RHElementTarget::GetCommand so a malformed
                // command/target pairing does not silently acquire this
                // route.
                // ENTER_SWORDFIGHT is likewise unambiguous: it is only ever
                // resolved by a soldier click, whose route is the classical
                // sword seek (tolerance = the PC's own sword range) plus the
                // VIP, table-swordfight and cross-gate forks. The generic
                // AddInteractionWithSeek helper has no sword-range entry and
                // would seek at the 30-unit interaction default instead,
                // stopping the PC short of — or past — the opponent.
                if *command == Command::EnterSwordfight {
                    self.apply_enter_swordfight(sim, assets, *actor, *target, *running);
                } else if matches!(
                    command,
                    Command::SearchCmd
                        | Command::UseLever
                        | Command::HitTarget
                        | Command::HandleTarget
                        | Command::TakeTarget
                        | Command::Pay
                ) && matches!(
                    self.get_entity(*target),
                    Some(crate::element::Entity::Target(_))
                ) && !self.players.qa_recording_for.contains(actor)
                {
                    if *running {
                        self.actor_make_fast(sim, *actor);
                    } else if self
                        .apply_target_interaction_route(sim, *actor, *target, *command, *running)
                    {
                        self.hero_speaking(
                            assets,
                            *actor,
                            crate::engine::melee::HERO_ACCEPT_COMMAND,
                        );
                    }
                } else {
                    self.apply_interaction_with_seek(sim, *actor, *target, *command, *running);
                }
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
                    self.stop_recording_macro();
                    return;
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
                // Purse/wasp/net ground-target handlers call
                // LaunchSequenceElement in the Original. Keep their owner
                // instruction at the post-entity manager boundary.
                let mut seq = Sequence::new();
                seq.append_element(elem);
                self.launch_sequence(seq);
            }
            LaunchSelfAbility { actor, command } => {
                if self.players.qa_recording_for.contains(actor) {
                    // The shared recorder already captured the step. Manual
                    // QA recording stores this ability instead of applying it
                    // to the live PC.
                    self.stop_recording_macro();
                    return;
                }
                let elem = SequenceElement::new(1, *command, Some(*actor));
                // The corresponding Original input handlers use
                // LaunchSequenceElement. Registration is immediate, but the
                // owner's Instruct boundary belongs to the manager pass after
                // this frame's entity loop; do not interrupt its current
                // order before that final Execute tick.
                let mut seq = Sequence::new();
                seq.append_element(elem);
                self.launch_sequence(seq);
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
                    self.stop_recording_macro();
                    return;
                }
                self.apply_scroll_read_with_seek(sim, *actor, *target, *running);
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
                with_seek,
                seek_distance,
            } => {
                tracing::trace!(
                    ?actor,
                    ?target,
                    ?command,
                    with_seek,
                    "PlayerCommand::SwordStrikeCmd"
                );
                self.prepare_allied_player_combat_command(*actor);
                if *with_seek {
                    self.apply_sword_strike_with_seek(
                        assets,
                        *actor,
                        *target,
                        *command,
                        *seek_distance,
                    );
                } else {
                    let elem =
                        SequenceElement::new_interaction(1, *command, Some(*actor), Some(*target));
                    // Original mouse-command handling calls
                    // SequenceManager::LaunchSequenceElement here.  A
                    // preference strike is therefore registered for the
                    // post-entity manager drain; it does not arbitrate
                    // against and interrupt the actor's current order on the
                    // input callback stack.
                    let mut sequence = Sequence::new();
                    sequence.append_element(elem);
                    self.launch_sequence(sequence);
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
                let selected_before = self.players.seats[seat].selection.clone();
                if self.select_pc_action_by_index(assets, input, seat, *pc_id, *action_index as u8)
                {
                    self.close_player_select_action_stop_callbacks(sim, assets, selected_before);
                }
            }
            SelectResolvedAction { pc_id, action } => {
                let selected_before = self.players.seats[seat].selection.clone();
                self.set_pc_action(assets, input, seat, *pc_id, *action);
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
            }
            CancelPlannedAction => {
                self.players.seats[seat].planned_action = crate::profiles::Action::NoAction;
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
                self.players.seats[seat].selected_action = crate::profiles::Action::NoAction;
            }
            MouseRightDown => {
                input.right_mouse_down = true;
            }
            MouseRightUp => {
                input.right_mouse_down = false;
            }
            ClearShootList { pc_id } => {
                // Clear Human::Instruct's retained pointer FIFO. Keep the
                // broader pending-element cleanup for pre-Instruct work that
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
                let mut elem = SequenceElement::new_generic(1, Command::DropAmmo, Some(*pc_id));
                elem.set_property(Field::ActionId, FieldValue::Integer(*action_id));
                elem.set_property(Field::Amount, FieldValue::Integer(*amount));
                // MSG_DROP_*_AMMO uses LaunchSequenceElement, not a direct
                // actor Instruct call.
                let mut seq = Sequence::new();
                seq.append_element(elem);
                self.launch_sequence(seq);
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
                self.apply_drop_ale_at(
                    *actor,
                    *target_pos,
                    *running,
                    *already_authorized,
                    *goal_override,
                    *goal_sector_index_override,
                    recorded_gate_path.clone(),
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
                if self.players.qa_recording_for.contains(actor) {
                    // The first shield click already selected the protectee.
                    // Original's second click updates the prompt state, stores
                    // the concrete Seek -> RaiseShield quick action, and stops
                    // recording without launching it against the live actor.
                    self.world.shield.is_protected = true;
                    self.world.shield.protected_pc = Some(*protected_pc);
                    self.world.shield.danger_point = *danger_point;
                    self.world.shield.danger_point_layer = *danger_point_layer;
                    self.stop_recording_macro();
                    return;
                }
                self.apply_raise_shield_with_danger(
                    *actor,
                    *protected_pc,
                    *danger_point,
                    *danger_point_layer,
                );
            }

            // ── Posture ─────────────────────────────────────────
            CrouchDown => self.apply_crouch_down(sim, seat),
            StandUp => self.apply_stand_up(sim, seat),

            // ── Selection ───────────────────────────────────────
            SelectPc { pc_id, append } => {
                if !append {
                    self.players.allied.ensure_seat(seat).selection.clear();
                }
                if recorded_nested_selection_action {
                    assert!(
                        self.get_entity(*pc_id)
                            .and_then(crate::element::Entity::pc_data)
                            .is_some(),
                        "recorded nested selection action targets missing or non-PC {pc_id:?}"
                    );
                }
                self.select_pc_with_action_fanout(
                    assets,
                    seat,
                    *pc_id,
                    *append,
                    true,
                    !recorded_nested_selection_action,
                );
                self.update_recording_after_selection_change();
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
                if !append {
                    self.players.allied.ensure_seat(seat).selection.clear();
                }
                // Portrait click → `select_by_portrait_index` fires
                // `select_pc` with `speak=true` directly.
                self.select_by_portrait_index(assets, seat, *portrait_index as u8, *append);
                self.update_recording_after_selection_change();
            }
            SelectAlliedSoldiers { soldiers, append } => {
                self.select_allied_soldiers(seat, soldiers, *append);
            }
            BoxSelectAlliedSoldiers { pt1, pt2, shift } => {
                self.box_select_allied_soldiers(seat, *pt1, *pt2, *shift);
            }
            ClearAlliedSelection => {
                self.players.allied.ensure_seat(seat).selection.clear();
            }
            PinAlliedSelection => self.pin_allied_selection(seat),
            UnpinAlliedGroup { group_id } => self.unpin_allied_group(seat, *group_id),
            SelectAlliedGroup { group_id, append } => {
                if !append {
                    self.unselect_all_pcs(seat);
                }
                self.select_allied_group(seat, *group_id, *append);
            }
            PageAlliedPortraits { delta } => self.page_allied_portraits(seat, *delta),
            MoveAlliedSoldiers {
                soldiers,
                destination,
                running,
                formation,
            } => {
                let leaders = self.players.seats[seat].selection.clone();
                self.command_allied_move(
                    sim,
                    assets,
                    soldiers,
                    &leaders,
                    *destination,
                    *running,
                    *formation,
                )
            }
            SetAlliedStance { soldiers, stance } => {
                self.set_allied_stance(soldiers, *stance);
            }
            SetAlliedFormation {
                soldiers,
                formation,
            } => self.set_allied_formation(soldiers, *formation),
            SetAlliedPatrol {
                soldiers,
                destination,
                formation,
            } => self.set_allied_patrol(sim, assets, soldiers, *destination, *formation),
            SetAlliedFollow {
                soldiers,
                hero,
                formation,
            } => self.set_allied_follow(assets, soldiers, *hero, *formation),
            ReleaseAlliedControl => self.release_allied_control(),

            // ── Special ─────────────────────────────────────────
            ResetComa { pc_id } => self.reset_coma(assets, *pc_id),
            SendReinforcement { pc_id } => self.request_reinforcement(*pc_id),
            // Use the actor-level MakeFast so the pathfinder + queued
            // transitions get rewritten, not just the element-level
            // action.
            MakePcFast { pc_id } => self.actor_make_fast(sim, *pc_id),
            BeggarDontTalkStamp { beggar_id } => self.stamp_beggar_dont_talk_counter(*beggar_id),
            MakePcSlow { pc_id } => self.actor_make_slow(sim, *pc_id),
            MakePcUpright { pc_id } => self.actor_make_upright(sim, *pc_id),
            MakePcCrouched { pc_id } => self.actor_make_crouched(sim, *pc_id),

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
                self.apply_start_macro(sim, display, input, assets, *pc, *slot);
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
                self.apply_queue_quick_action(sim, display, input, assets, seat, *action, command);
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
            RefreshSelectedPatchDisplayDoors { selected_patch_idx } => {
                self.refresh_selected_patch_display_doors(*selected_patch_idx);
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
            CampaignConvertSelectedPeasantsToBlazons => {
                self.convert_selected_peasants_to_blazons(sim, &assets.profile_manager);
            }
            ApplyQuitMissionUpdates {
                exit_code,
                difficulty,
            } => {
                self.apply_quit_mission_updates(sim, assets, *exit_code, *difficulty);
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
            MinimapMouseDown {
                click_pt,
                continuing_drag,
            } => {
                // Begin dragging on LEFTDOWN inside widget when the map
                // is deployed.
                if display.minimap.is_displayed() {
                    let screen = Self::director_camera_view_size();
                    let sw = screen.x;
                    let sh = screen.y;
                    display.minimap.manage_dragging(*click_pt, sw, sh);
                }
                // The host resolves this before dispatch. Do not infer an
                // engine message from rollback-local minimap scratch.
                if *continuing_drag {
                    self.orders.messenger.send(crate::messenger::Message::new(
                        crate::messenger::MessageType::Simple(
                            crate::messenger::SimpleMessage::UiHasFocus,
                        ),
                    ));
                    input.has_focus = false;
                }
            }
            MinimapMouseMove {
                mouse_pt,
                left_mouse_down,
                continuing_drag,
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
                }
                if *continuing_drag {
                    // Continuing-drag focus is command-derived. The host
                    // presentation mutation above may legitimately differ
                    // on a rollback scratch display.
                    self.orders.messenger.send(crate::messenger::Message::new(
                        crate::messenger::MessageType::Simple(
                            crate::messenger::SimpleMessage::UiHasFocus,
                        ),
                    ));
                    input.has_focus = false;
                }
            }
            MinimapMouseUp {
                on_minimap,
                center_on,
            } => {
                // Check the dragged flag, dead zone, and dispatch to
                // open-map or center-on-click.
                display.minimap.drag_start = false;
                if !display.minimap.dragged {
                    if *on_minimap {
                        if !display.minimap.is_displayed() {
                            display.minimap.manage_click();
                        }
                    }
                } else {
                    display.minimap.dragged = false;
                }
                display.minimap.close_after_highlight = false;

                if let Some(world_pt) = center_on {
                    let level_size = self.feedback.cutscene_camera.level_size;
                    assert!(
                        world_pt.x.is_finite()
                            && world_pt.y.is_finite()
                            && world_pt.x >= 0.0
                            && world_pt.y >= 0.0
                            && world_pt.x <= level_size.x
                            && world_pt.y <= level_size.y,
                        "MinimapMouseUp center_on point ({}, {}) is outside required level bounds ({}, {})",
                        world_pt.x,
                        world_pt.y,
                        level_size.x,
                        level_size.y
                    );
                    // The zoom gate and locker state are both Engine-owned;
                    // only the projected point crosses the command boundary.
                    if self.is_camera_zoom_possible_for_seat(seat) {
                        self.players.seats[seat].locker_active = false;
                        self.center_on_point(seat, *world_pt);
                    }
                }
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

    /// Close the `RHElementActor::Stop` cards authored by a player action
    /// selection before the next `RHEngine::Hourglass` actor walk.
    ///
    /// Original `RHEngine::SelectAction` calls `Stop()` synchronously for
    /// every selected PC (`original-code/RHengine.cpp:13054-13086`).  A
    /// stopped `TakeCorpse` can therefore run its condolence immediately:
    /// `DropCorpse(12, true)` releases the body and calls the body's `Wait()`
    /// before creation-order actor slots begin
    /// (`original-code/RHelementactorpc.cpp:6476-6506`).  Rust queues cards
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

    /// Append a `QuickActionStep` to the currently-recording PC's
    /// macro, if a recording is in progress and the command targets
    /// that PC.  No-op otherwise.
    ///
    /// Only the `Action` + target `position` is stored per step — the
    /// per-slot titbit id is set separately at the `AddTitbit` site.
    fn record_macro_step_for(&mut self, seat: usize, cmd: &PlayerCommand, assets: &LevelAssets) {
        if self.players.qa_recording_for.is_empty() {
            return;
        }
        // When multiple PCs are armed for recording, each one receives
        // its own macro step.  Snapshot the set up-front so we can
        // re-borrow `self` inside the per-PC loop.
        let recording_pcs = self.players.qa_recording_for.clone();
        for recording_pc in recording_pcs {
            self.record_macro_step_for_pc(
                seat,
                cmd,
                recording_pc,
                None,
                assets,
                QuickActionRecordingStore::Manual,
            );
        }
    }

    fn record_macro_step_for_pc(
        &mut self,
        seat: usize,
        cmd: &PlayerCommand,
        recording_pc: EntityId,
        action_override: Option<crate::profiles::Action>,
        assets: &LevelAssets,
        recording_store: QuickActionRecordingStore,
    ) {
        use crate::macro_store::QuickActionStep;
        use PlayerCommand::*;

        // Helper: read the acting PC's current action.  Returns
        // NoAction if the entity isn't a PC or doesn't exist.
        let pc_action = |engine: &EngineInner, pc: EntityId| -> crate::profiles::Action {
            action_override.unwrap_or_else(|| {
                engine
                    .get_entity(pc)
                    .and_then(|e| e.pc_data())
                    .map(|pc| pc.current_action)
                    .unwrap_or(crate::profiles::Action::NoAction)
            })
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
                goal_sector_index_override: _,
                door_route_override: _,
                recorded_gate_routes: _,
                recorded_failed_gate_routes: _,
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
                let Some(target_entity) = self.get_entity(*target) else {
                    return;
                };
                let target_pos = target_entity.element_data().position_map();
                let action = pc_action(self, *actor);
                running_move = *running;
                let replay = if matches!(target_entity, crate::element::Entity::Target(_))
                    && matches!(
                        command,
                        Command::SearchCmd
                            | Command::UseLever
                            | Command::HitTarget
                            | Command::HandleTarget
                            | Command::TakeTarget
                            | Command::Pay
                    ) {
                    let movement_action = if *running {
                        crate::order::OrderType::RunningUpright
                    } else if self.get_entity(*actor).is_some_and(|entity| {
                        entity.element_data().posture == crate::element::Posture::Crouched
                    }) {
                        crate::order::OrderType::WalkingCrouched
                    } else {
                        crate::order::OrderType::WalkingUpright
                    };
                    QaReplayCommand::TargetInteraction {
                        target: *target,
                        command: *command,
                        destination: target_pos,
                        sector: target_entity.element_data().sector(),
                        layer: target_entity.element_data().layer(),
                        action: movement_action,
                        turn_point: target_entity.cxx_current_point_map().unwrap_or_else(|| {
                            panic!("recorded target interaction {target:?} has no current point")
                        }),
                    }
                } else {
                    QaReplayCommand::Interaction {
                        target: *target,
                        command: *command,
                        // The double-click bit is the same bit that
                        // drives `running=true` on the input side, so
                        // reuse it as our recorded double-click flag.
                        double_click: *running,
                    }
                };
                (*actor, action, target_pos, replay)
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
                let pos = MapPoint::new(target_pos.x, target_pos.y - target_pos.z);
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
                already_authorized: _,
                goal_override: _,
                goal_sector_index_override: _,
                recorded_gate_path: _,
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
                seek_distance,
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
                        seek_distance: *seek_distance,
                    },
                )
            }
            RaiseShieldWithDanger {
                actor,
                protected_pc,
                danger_point,
                danger_point_layer,
            } => {
                if *actor != recording_pc {
                    return;
                }
                if self.get_entity(*protected_pc).is_none() {
                    panic!(
                        "recorded shield protectee {protected_pc:?} disappeared before QA registration"
                    );
                }
                phase_override = Some(crate::titbit::QuickAction::Shield);
                (
                    *actor,
                    pc_action(self, *actor),
                    danger_point.to_map(),
                    QaReplayCommand::ShieldRaise {
                        protected_pc: *protected_pc,
                        danger_point: *danger_point,
                        danger_point_layer: *danger_point_layer,
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

        let step = QuickActionStep {
            action,
            position,
            replay,
        };
        let slot_idx =
            match recording_store {
                QuickActionRecordingStore::Manual => {
                    if let Some(replaced_slot) = self.players.macro_store.get(actor).and_then(
                        crate::macro_store::PcMacroState::recording_replaces_existing_slot,
                    ) {
                        self.remove_quick_action_titbits_for(actor, replaced_slot);
                        self.players
                            .macro_store
                            .get_mut(actor)
                            .expect("recording macro state disappeared while replacing a slot")
                            .clear_slot_titbit(usize::from(replaced_slot));
                    }
                    self.players.macro_store.append(actor, step);
                    self.players
                        .macro_store
                        .get(recording_pc)
                        .and_then(crate::macro_store::PcMacroState::recording_slot)
                        .map(usize::from)
                }
                QuickActionRecordingStore::Automatic => {
                    self.players.auto_queues.push(actor, step);
                    Some(self.players.auto_queues.len(actor) - 1)
                }
            };

        // Register a QuickAction titbit once per macro slot and feed
        // the id into the slot.
        let Some(slot_idx) = slot_idx else {
            return;
        };
        if recording_store == QuickActionRecordingStore::Manual
            && self
                .players
                .macro_store
                .get(recording_pc)
                .and_then(|state| state.get_slot_titbit(slot_idx))
                .is_some()
        {
            return;
        }
        let phase = match (phase_override, replay) {
            (Some(q), _) => q as u16,
            (
                None,
                QaReplayCommand::Interaction {
                    target, command, ..
                }
                | QaReplayCommand::TargetInteraction {
                    target, command, ..
                },
            ) => {
                let target_entity = self.get_entity(target).unwrap_or_else(|| {
                    panic!("quick-action interaction target {target:?} disappeared")
                });
                if command == Command::Take
                    && matches!(
                        target_entity,
                        crate::element::Entity::Bonus(_)
                            | crate::element::Entity::Scroll(_)
                            | crate::element::Entity::Projectile(_)
                            | crate::element::Entity::Net(_)
                    )
                {
                    crate::titbit::QuickAction::Take as u16
                } else if let crate::element::Entity::Target(target) = target_entity {
                    let pc_char_profile = self
                        .get_entity(actor)
                        .and_then(|entity| entity.pc_data())
                        .and_then(|pc| assets.profile_manager.get_character(pc.profile_index));
                    let pc_has_search = pc_char_profile
                        .is_some_and(|profile| profile.has_contextual_action(Action::Search));
                    let pc_is_vip = self
                        .get_entity(actor)
                        .is_some_and(|entity| self.is_entity_vip(assets, entity));
                    super::target_interaction::target_qa_titbit(
                        target.target.action_filter,
                        pc_has_search,
                        pc_is_vip,
                    )
                } else {
                    action_to_quick_phase(action, running_move) as u16
                }
            }
            (None, _) => action_to_quick_phase(action, running_move) as u16,
        };
        // Original attaches entity-target QAs to their target supplier, but
        // stores movement/ground QAs as fixed 3D points. The distinction is
        // also a rendering contract: supplier icons float above the entity;
        // fixed-point crosshairs are centered directly on the destination.
        let supplier = match replay {
            QaReplayCommand::Interaction { target, .. }
            | QaReplayCommand::TargetInteraction { target, .. }
            | QaReplayCommand::ScrollRead { target, .. }
            | QaReplayCommand::Swordfight { target, .. }
            | QaReplayCommand::SwordStrike { target, .. }
            | QaReplayCommand::ShieldRaise {
                protected_pc: target,
                ..
            } => Some(target),
            QaReplayCommand::SelfAbility { .. } | QaReplayCommand::PostureToggle { .. } => {
                Some(actor)
            }
            QaReplayCommand::Move { .. }
            | QaReplayCommand::GroundTarget { .. }
            | QaReplayCommand::DropAle { .. } => None,
        };
        let actor_layer = self
            .get_entity(recording_pc)
            .map(|entity| entity.element_data().layer())
            .unwrap_or_else(|| panic!("quick-action recording PC {recording_pc:?} disappeared"));
        let (pos3d, layer) = match replay {
            QaReplayCommand::GroundTarget {
                target_pos,
                titbit_layer,
                ..
            } => (target_pos, titbit_layer),
            QaReplayCommand::ShieldRaise {
                danger_point,
                danger_point_layer,
                ..
            } => (danger_point, danger_point_layer),
            QaReplayCommand::Move { destination, .. }
            | QaReplayCommand::DropAle {
                target_pos: destination,
                ..
            } => (
                self.world.fast_grid.convert_2d_to_3d(
                    destination,
                    crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
                    self.sight_obstacles(assets),
                ),
                actor_layer,
            ),
            _ => (
                crate::coordinates::WorldPoint3D::new(0.0, 0.0, 0.0),
                supplier
                    .and_then(|id| self.get_entity(id))
                    .map(|entity| entity.element_data().layer())
                    .unwrap_or(actor_layer),
            ),
        };
        let manager = ElementHandle(recording_pc.index());
        let supplier_handle = supplier
            .map(|id| ElementHandle(id.index()))
            .unwrap_or(ElementHandle::INVALID);
        let titbit_id = self.feedback.titbit_manager.add_titbit(
            pos3d,
            layer,
            TitbitKind::QuickAction,
            supplier_handle,
            phase,
            manager,
            running_move, // Run companion titbit
            INVALID_ID,
            true,
            None,
            Some(layer),
        );
        if let Some(tb) = crate::titbit::TitbitId::new(titbit_id) {
            match recording_store {
                QuickActionRecordingStore::Manual => self
                    .players
                    .macro_store
                    .get_mut(recording_pc)
                    .unwrap_or_else(|| {
                        panic!("manual quick-action state for {recording_pc:?} disappeared")
                    })
                    .set_slot_titbit(slot_idx, tb),
                QuickActionRecordingStore::Automatic => {
                    self.players.auto_queues.set_last_titbit(recording_pc, tb)
                }
            }
        }
    }

    /// Append a Shift-click action to each addressed PC's automatic queue.
    /// Recording is performed directly against `AutoQueueStore`: the nested live
    /// command is never applied here, which is what keeps action planning from
    /// equipping weapons or interrupting an actor that is still moving.
    fn apply_queue_quick_action(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        seat: usize,
        action: crate::profiles::Action,
        command: &PlayerCommand,
    ) {
        use PlayerCommand::*;

        let actors: Vec<EntityId> = match command {
            GroupMove { actors, .. } => actors.clone(),
            LaunchInteraction { actor, .. }
            | LaunchGroundTarget { actor, .. }
            | DropAleAt { actor, .. }
            | LaunchSelfAbility { actor, .. }
            | LaunchScrollRead { actor, .. }
            | EnterSwordfight { actor, .. }
            | SwordStrikeCmd { actor, .. } => vec![*actor],
            CrouchDown | StandUp => self.players.seats[seat].selection.clone(),
            _ => {
                tracing::warn!(?command, "Shift queue rejected unsupported player command");
                return;
            }
        };

        for actor in actors {
            let before_len = self.players.auto_queues.len(actor);
            self.record_macro_step_for_pc(
                seat,
                command,
                actor,
                Some(action),
                assets,
                QuickActionRecordingStore::Automatic,
            );

            if self.players.auto_queues.len(actor) != before_len + 1 {
                tracing::warn!(?actor, ?command, "Shift queue command produced no QA step");
                continue;
            }

            let was_active = self.players.auto_queue_active.contains(&actor);
            if !was_active {
                self.players.auto_queue_active.push(actor);
            }
            let actor_busy = self
                .orders
                .sequence_manager
                .has_unpostponed_element_for_actor_matching(actor, |command| {
                    command != Command::Wait
                });
            if !was_active && !actor_busy {
                self.start_auto_queue_front(sim, display, input, assets, actor);
            }
        }
    }

    fn apply_make_queued_action_fast(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        pc: EntityId,
    ) {
        let upgraded_slot = self.players.auto_queues.make_last_move_running(pc);
        if let Some(slot) = upgraded_slot {
            let titbit = self
                .players
                .auto_queues
                .get(pc)
                .and_then(|queue| queue.get(slot))
                .and_then(|entry| entry.titbit)
                .unwrap_or_else(|| panic!("queued run PC {pc:?} slot {slot} has no titbit"));
            self.feedback
                .titbit_manager
                .promote_quick_action_to_run(titbit);
        } else {
            self.actor_make_fast(sim, pc);
        }
    }

    /// Launch the front QA for one automatic queue and immediately collapse
    /// that PC's memory strip. The launched sequence is now the active item;
    /// the visible slots contain only work still waiting behind it.
    fn start_auto_queue_front(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        pc: EntityId,
    ) {
        let Some(entry) = self
            .players
            .auto_queues
            .get(pc)
            .and_then(|queue| queue.first())
            .cloned()
        else {
            return;
        };
        let launched = self
            .check_quick_action_steps_validity(pc, std::slice::from_ref(&entry.step))
            && self.replay_quick_action_steps(
                sim,
                display,
                input,
                assets,
                pc,
                vec![entry.step.clone()],
            );
        if !launched {
            // Automatic queues cannot wait for a user to click a failed QA
            // item. Fizzle once, discard the invalid front item, and leave
            // the tail ready to advance on the next idle tick.
            tracing::warn!(?pc, "automatic quick action fizzled; dropping queue front");
            self.feedback
                .pending_side_effects
                .sounds
                .push(super::SoundCommand::Jingle(
                    crate::sound::Jingle::QuickActionFailed,
                ));
        }
        if let Some(titbit) = entry.titbit {
            self.feedback
                .titbit_manager
                .remove_quick_action_titbits_by_id(titbit);
        }
        let retired = self
            .players
            .auto_queues
            .pop_front(pc)
            .unwrap_or_else(|| panic!("automatic quick-action queue for {pc:?} disappeared"));
        assert_eq!(
            retired, entry,
            "automatic queue front changed during replay"
        );
        // TODO(ui): give the independent auto-queue strip its own falling
        // animation state. The deterministic queue contents already shift
        // here; only the cosmetic easing remains shared with manual QA UI.
        let _ = display;
    }

    /// Advance automatic Shift-click queues after actor work has settled for
    /// the frame. Called by the engine tick, not the renderer, so replay and
    /// rollback observe identical launch frames.
    pub(super) fn advance_auto_quick_action_queues(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
    ) {
        let active = self.players.auto_queue_active.clone();
        let mut scratch_input = InputState::default();
        for pc in active {
            if self
                .orders
                .sequence_manager
                .has_unpostponed_element_for_actor_matching(pc, |command| command != Command::Wait)
            {
                continue;
            }
            if !self.players.auto_queues.is_empty(pc) {
                self.start_auto_queue_front(sim, display, &mut scratch_input, assets, pc);
            } else {
                self.players
                    .auto_queue_active
                    .retain(|queued| *queued != pc);
            }
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
        sim: &crate::sim_rng::SimulationContext,
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
                .world
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
            self.replay_macro_slot(sim, display, input, assets, *pc_id, slot);
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
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        pc: EntityId,
        slot: u8,
    ) {
        if self
            .replay_legacy_quickito(sim, display, assets, pc, slot)
            .is_some()
        {
            return;
        }
        if self
            .replay_legacy_sequence_macro(assets, pc, slot)
            .is_some()
        {
            return;
        }
        // Pre-flight: if any recorded element fails its per-element
        // gate, the entire macro is rejected and the slot is preserved
        // so the player can retry.  The replay walks per-step rather
        // than rebuilding one sequence, so we run the gate once up
        // front and bail without dispatching or clearing on failure —
        // the jingle path in `apply_start_macro` then keys off the
        // slot still being occupied to emit `QuickActionFailed`.
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

        if !self.check_quick_action_steps_validity(pc, &steps)
            || !self.replay_quick_action_steps(sim, display, input, assets, pc, steps)
        {
            return;
        }

        // Drop the manual slot's titbit and clear only that manual slot.
        self.remove_quick_action_titbits_for(pc, slot);
        if let Some(state) = self.players.macro_store.get_mut(pc) {
            state.clear_slot(slot as usize);
        }
    }

    /// Dispatch already-snapshotted QA steps without making assumptions
    /// about whether they came from a manual macro or the automatic queue.
    /// Returns false when the sequence fizzles and the caller must decide how
    /// to retire its own storage.
    fn replay_quick_action_steps(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        input: &mut InputState,
        assets: &LevelAssets,
        pc: EntityId,
        steps: Vec<crate::macro_store::QuickActionStep>,
    ) -> bool {
        let step_count = steps.len();
        let mut posture_recovery_embedded = false;
        for (step_index, step) in steps.into_iter().enumerate() {
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
                    goal_sector_index_override: None,
                    door_route_override: None,
                    recorded_gate_routes: Vec::new(),
                    recorded_failed_gate_routes: Vec::new(),
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
                        return false;
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
                        self.apply_command(sim, display, input, assets, &pre_click);
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
                crate::macro_store::QaReplayCommand::TargetInteraction {
                    target,
                    command,
                    destination,
                    sector,
                    layer,
                    action,
                    turn_point,
                } => {
                    if self.get_entity(target).is_none() {
                        return false;
                    }
                    self.replay_recorded_target_interaction(
                        pc,
                        target,
                        command,
                        destination,
                        sector,
                        layer,
                        action,
                        turn_point,
                    );
                    continue;
                }
                crate::macro_store::QaReplayCommand::ScrollRead { target, running } => {
                    // See Interaction arm — whole-sequence abort on
                    // target-gone.
                    if self.get_entity(target).is_none() {
                        return false;
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
                    already_authorized: false,
                    goal_override: None,
                    goal_sector_index_override: None,
                    recorded_gate_path: None,
                },
                crate::macro_store::QaReplayCommand::Swordfight { target, running } => {
                    // See Interaction arm — whole-sequence abort on
                    // target-gone.
                    if self.get_entity(target).is_none() {
                        return false;
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
                    seek_distance,
                } => {
                    // See Interaction arm — whole-sequence abort on
                    // target-gone.
                    if self.get_entity(target).is_none() {
                        return false;
                    }
                    PlayerCommand::SwordStrikeCmd {
                        actor: pc,
                        target,
                        command,
                        with_seek,
                        seek_distance,
                    }
                }
                crate::macro_store::QaReplayCommand::ShieldRaise {
                    protected_pc,
                    danger_point,
                    danger_point_layer,
                } => {
                    if self.get_entity(protected_pc).is_none() {
                        return;
                    }
                    PlayerCommand::RaiseShieldWithDanger {
                        actor: pc,
                        protected_pc,
                        danger_point,
                        danger_point_layer,
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
                        self.actor_make_crouched(sim, pc);
                    } else {
                        let posture = self
                            .get_entity(pc)
                            .map(|e| e.element_data().posture)
                            .unwrap_or(crate::element::Posture::Upright);
                        match posture {
                            crate::element::Posture::Crouched => {
                                self.actor_make_upright(sim, pc);
                            }
                            crate::element::Posture::SimulatingBeggar => {
                                let elem = SequenceElement::new(1, Command::LeaveBeggar, Some(pc));
                                let mut sequence = Sequence::new();
                                sequence.append_element(elem);
                                self.launch_sequence(sequence);
                            }
                            crate::element::Posture::Spy
                            | crate::element::Posture::AnonymousArcher => {
                                let elem = SequenceElement::new(1, Command::LeaveSpy, Some(pc));
                                let mut sequence = Sequence::new();
                                sequence.append_element(elem);
                                self.launch_sequence(sequence);
                            }
                            crate::element::Posture::Tree => {
                                let elem = SequenceElement::new(1, Command::LeaveTree, Some(pc));
                                let mut sequence = Sequence::new();
                                sequence.append_element(elem);
                                self.launch_sequence(sequence);
                            }
                            _ => {}
                        }
                    }
                    continue;
                }
            };
            // Original StartQuickAction clones the recorded elements into a
            // single sequence and appends posture recovery to that sequence
            // before launching it.  Keep the final TakeCorpse interaction
            // and its recovery in the same route: a standalone recovery is a
            // competing root and can otherwise win arbitration first.
            // TODO(parity): coalesce every modern QuickActionStep variant
            // into one Original-shaped action/post-seek sequence.  Those
            // variants currently dispatch through heterogeneous builders
            // with command-specific side effects, so only the source-proven
            // final TakeCorpse and DropAle shapes are embedded here.
            if step_index + 1 == step_count
                && let PlayerCommand::LaunchInteraction {
                    actor,
                    target,
                    command: Command::TakeCorpse,
                    running,
                } = &cmd
            {
                self.apply_interaction_with_seek_and_recovery(
                    sim,
                    *actor,
                    *target,
                    Command::TakeCorpse,
                    *running,
                    true,
                );
                posture_recovery_embedded = true;
            } else if step_index + 1 == step_count
                && let PlayerCommand::DropAleAt {
                    actor,
                    target_pos,
                    running,
                    already_authorized,
                    goal_override,
                    goal_sector_index_override,
                    recorded_gate_path,
                } = &cmd
            {
                self.apply_drop_ale_at_with_recovery(
                    *actor,
                    *target_pos,
                    *running,
                    true,
                    *already_authorized,
                    *goal_override,
                    *goal_sector_index_override,
                    recorded_gate_path.clone(),
                );
                posture_recovery_embedded = true;
            } else {
                self.apply_command(sim, display, input, assets, &cmd);
            }
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
        if !posture_recovery_embedded {
            let mut recovery = crate::sequence::Sequence::default();
            self.append_posture_recovery(pc, &mut recovery);
            if !recovery.elements.is_empty() {
                self.launch_sequence(recovery);
            }
        }

        // Post-seek continuation is now ported via
        // `ActorData::post_seek_sequence`: seek-building helpers attach
        // their continuation directly to the launched movement element,
        // so replay does not need an extra per-PC handoff here.

        true
    }

    /// Replay the three non-sequence quick-action variants serialized by
    /// `RHElementActorPC`. Interact deliberately re-enters the target's
    /// state-driven click ladder instead of guessing a resolved command from
    /// the saved target kind.
    fn replay_legacy_quickito(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut HostDisplayState,
        assets: &LevelAssets,
        pc: EntityId,
        slot: u8,
    ) -> Option<bool> {
        let quickito = self
            .players
            .macro_store
            .get(pc)?
            .slot(slot as usize)?
            .legacy_quickito()?;
        let succeeded = match quickito.kind {
            crate::element_kinds::QuickAction::None => {
                panic!("legacy Quickito slot contains QuickAction::None")
            }
            crate::element_kinds::QuickAction::GoDown => {
                self.actor_make_crouched(sim, pc);
                true
            }
            crate::element_kinds::QuickAction::GoUp => {
                self.actor_make_upright(sim, pc);
                true
            }
            crate::element_kinds::QuickAction::Interact => {
                let target = quickito
                    .interactor
                    .unwrap_or_else(|| panic!("legacy Interact Quickito has no interactor"));
                let succeeded = self.legacy_human_mouse_clicked(sim, assets, pc, target, false);
                if succeeded && quickito.button == 0x0008 {
                    // Original inserts a literal SequenceManager::Hourglass
                    // between the synthetic leading single-click and the
                    // saved double-click. At this input boundary no entity
                    // phase work remains; the normal sequence phase drains
                    // precisely the newly registered click sequence.
                    self.hourglass_phase_sequences(sim, display, assets);
                    self.actor_make_fast(sim, pc);
                }
                succeeded
            }
        };
        if !succeeded {
            return Some(false);
        }

        self.remove_quick_action_titbits_for(pc, slot);
        self.players
            .macro_store
            .get_mut(pc)
            .expect("legacy Quickito macro state disappeared")
            .clear_slot(slot as usize);
        let saved_pc = self
            .get_entity_mut(pc)
            .and_then(|entity| entity.pc_data_mut())
            .unwrap_or_else(|| panic!("legacy Quickito owner {pc:?} is not a PC"));
        saved_pc.quick_action_types[slot as usize] = crate::element_kinds::QuickAction::None;
        saved_pc.quick_action_buttons[slot as usize] = 0;
        saved_pc.quick_action_interactors[slot as usize] = None;
        Some(true)
    }

    /// Dedicated virtual-click equivalent for saved `QUICKITOS_INTERRACT`.
    /// The Original recorder only creates this variant for Human targets.
    fn legacy_human_mouse_clicked(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        pc: EntityId,
        target: EntityId,
        running: bool,
    ) -> bool {
        let target_entity = self
            .get_entity(target)
            .unwrap_or_else(|| panic!("legacy Quickito interactor {target:?} is missing"));
        assert!(
            target_entity.is_human(),
            "legacy Quickito interactor {target:?} is not Human"
        );
        let has_scroll = match target_entity {
            crate::element::Entity::Soldier(soldier) => soldier.npc.attached_scroll.is_some(),
            crate::element::Entity::Civilian(civilian) => civilian.npc.attached_scroll.is_some(),
            _ => false,
        };
        if has_scroll {
            self.apply_scroll_read_with_seek(sim, pc, target, running);
            return true;
        }
        let Some(command) = determine_use_command(self, assets, pc, target) else {
            return false;
        };
        self.apply_interaction_with_seek(sim, pc, target, command, running);
        true
    }

    /// Launch an exact owner-local quick-action sequence restored from an
    /// Original save. `Some(false)` preserves the slot after a validity
    /// failure; `Some(true)` consumed it; `None` selects normal semantic-step
    /// playback.
    fn replay_legacy_sequence_macro(
        &mut self,
        assets: &LevelAssets,
        pc: EntityId,
        slot: u8,
    ) -> Option<bool> {
        let (mut action, mut seek) = self
            .players
            .macro_store
            .get(pc)?
            .slot(slot as usize)?
            .legacy_sequences()
            .map(|(action, seek)| (action.clone(), seek.cloned()))?;
        let swordfighting = self
            .get_entity(pc)
            .and_then(|entity| entity.human_data())
            .is_some_and(|human| !human.opponents.is_empty());
        fn valid(
            engine: &EngineInner,
            assets: &LevelAssets,
            sequence: &crate::sequence::Sequence,
            swordfighting: bool,
            is_seek: bool,
        ) -> bool {
            sequence.elements.iter().all(|element| {
                if swordfighting
                    && if is_seek {
                        element.command != Command::SpeakHeroReachDestination
                    } else {
                        !matches!(element.command, Command::Move | Command::Seek)
                    }
                {
                    return false;
                }
                let owner = element
                    .owner
                    .unwrap_or_else(|| panic!("legacy QA element {} has no owner", element.id));
                let Some(entity) = engine.get_entity(owner) else {
                    return false;
                };
                if entity.is_pc()
                    && !engine.check_sequence_element_validity(assets, owner, element, false)
                {
                    return false;
                }
                if element.command == Command::Seek
                    && let crate::sequence::SequenceElementData::Movement {
                        post_seek_sequence: Some(post_seek),
                        ..
                    } = &element.data
                    && !valid(engine, assets, post_seek, swordfighting, false)
                {
                    return false;
                }
                true
            })
        }
        if action.is_empty()
            || !valid(self, assets, &action, swordfighting, false)
            || seek
                .as_ref()
                .is_some_and(|sequence| !valid(self, assets, sequence, swordfighting, true))
        {
            return Some(false);
        }

        for element in &mut action.elements {
            element.script_driven = true;
            element.orders.clear();
            element.num_transition_orders = 0;
            element.retained_movement_goal = None;
            element.cross_postponed = None;
        }
        if let Some(sequence) = &mut seek {
            for element in &mut sequence.elements {
                element.script_driven = true;
                element.orders.clear();
                element.num_transition_orders = 0;
                element.retained_movement_goal = None;
                element.cross_postponed = None;
            }
            self.append_posture_recovery(pc, sequence);
        } else {
            self.append_posture_recovery(pc, &mut action);
        }

        if let Some(seek) = seek {
            let actor = self
                .get_entity_mut(pc)
                .and_then(|entity| entity.actor_data_mut())
                .unwrap_or_else(|| panic!("legacy QA owner {pc:?} is not an actor"));
            actor.post_seek_sequence = Some(Box::new(seek));
        }
        self.remove_quick_action_titbits_for(pc, slot);
        self.launch_sequence(action);
        self.players
            .macro_store
            .get_mut(pc)
            .expect("legacy QA macro state disappeared")
            .clear_slot(slot as usize);
        let saved_pc = self
            .get_entity_mut(pc)
            .and_then(|entity| entity.pc_data_mut())
            .unwrap_or_else(|| panic!("legacy QA owner {pc:?} is not a PC"));
        saved_pc.quick_action_sequences[slot as usize] = None;
        saved_pc.quick_seek_sequences[slot as usize] = None;
        saved_pc.quick_action_special_counts[slot as usize] = 0;
        Some(true)
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
    fn check_quick_action_steps_validity(
        &self,
        pc: EntityId,
        steps: &[crate::macro_store::QuickActionStep],
    ) -> bool {
        use crate::macro_store::QaReplayCommand;
        if steps.is_empty() {
            return false;
        }
        let is_swordfighting = self
            .get_entity(pc)
            .and_then(|e| e.human_data())
            .map(|h| !h.opponents.is_empty())
            .unwrap_or(false);
        for step in steps {
            let target = match &step.replay {
                QaReplayCommand::Interaction { target, .. }
                | QaReplayCommand::TargetInteraction { target, .. }
                | QaReplayCommand::ScrollRead { target, .. }
                | QaReplayCommand::Swordfight { target, .. }
                | QaReplayCommand::SwordStrike { target, .. }
                | QaReplayCommand::ShieldRaise {
                    protected_pc: target,
                    ..
                } => Some(target),
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
        let targets = match pc {
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
            if self
                .get_entity(*id)
                .and_then(|entity| entity.pc_data())
                .is_none()
            {
                panic!("quick-action recording target {id:?} is not a PC");
            }
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
        self.stop_recording_macro();
        // Re-arm on whoever is currently selected.
        let targets: Vec<EntityId> = self.players.seats[seat].selection.iter().copied().collect();
        if targets.is_empty() {
            return;
        }
        for id in &targets {
            self.players
                .macro_store
                .get_or_insert(*id)
                .begin_recording(slot);
            if self
                .get_entity(*id)
                .and_then(|entity| entity.pc_data())
                .is_none()
            {
                panic!("quick-action recording target {id:?} is not a PC");
            }
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
                let pcs = self.world.pc_ids.clone();
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
        self.world
            .fast_grid
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
        let distance = match command_action_distance_animation(command) {
            Some(animation) => self.actor_action_distance(actor, animation),
            None => Some(interaction_distance(command)),
        }?;
        // These Original input paths explicitly cast GetActionDistance to
        // UWORD before constructing AddInteractionWithSeek. Other
        // action-distance paths (notably DropAle and ClimbUpOnShoulders)
        // intentionally retain their fractional value.
        if matches!(
            command,
            Command::StrangleCmd
                | Command::HealCmd
                | Command::HitCmd
                | Command::UseLever
                | Command::WakeUp
                | Command::TakeCorpse
                | Command::SearchCmd
                | Command::TieCmd
        ) {
            Some((distance as u16) as f32)
        } else {
            Some(distance)
        }
    }

    /// Launch an interaction, prepending a Seek walk if the actor is
    /// too far away or in a different sector.
    fn apply_interaction_with_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        actor: EntityId,
        target: EntityId,
        command: Command,
        running: bool,
    ) {
        self.apply_interaction_with_seek_and_recovery(sim, actor, target, command, running, false);
    }

    fn stamp_beggar_dont_talk_counter(&mut self, target: EntityId) {
        // The Pay click resolver has already validated that this target is the
        // eligible beggar. Preserve the existing direct Civilian + FriendlyAi
        // mutation semantics here.
        let entity = self.get_entity_mut(target).unwrap_or_else(|| {
            panic!("beggar cooldown stamp target {} is missing", target.index())
        });
        let crate::element::Entity::Civilian(civilian) = entity else {
            panic!(
                "beggar cooldown stamp target {} is not a civilian",
                target.index()
            )
        };
        let crate::element::AiBrain::Friendly(ai) = &mut civilian.npc.ai_brain else {
            panic!(
                "beggar cooldown stamp target {} has no friendly AI",
                target.index()
            )
        };
        ai.set_beggar_dont_talk_counter(3);
    }

    /// Build the ordinary interaction route, optionally retaining quick-action
    /// posture recovery in the same sequence as the recorded interaction.
    fn apply_interaction_with_seek_and_recovery(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        actor: EntityId,
        target: EntityId,
        command: Command,
        running: bool,
        append_posture_recovery: bool,
    ) {
        // Ranged actions bypass the seek entirely: the actor fires or
        // throws from wherever it stands.  This mirrors the original
        // bow click path, which launches RHCOMMAND_SHOOT_BOW directly.
        if matches!(
            command,
            Command::ShootBow | Command::ShootBowOnce | Command::ThrowApple | Command::ThrowStone
        ) {
            let elem = SequenceElement::new_interaction(1, command, Some(actor), Some(target));
            // Original's input handlers call LaunchSequenceElement here.
            // That only registers the element for SequenceManager::Hourglass,
            // after the entity loop, so an order already owned by the PC gets
            // one final Execute tick before this interaction is instructed.
            let mut seq = Sequence::new();
            seq.append_element(elem);
            self.launch_sequence(seq);
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
            self.actor_make_fast(sim, actor);
            // Civilian::MouseClicked performs this post-call stamp even when
            // AddInteractionWithSeek reduced the double-click to MakeFast.
            if command == Command::Pay {
                self.stamp_beggar_dont_talk_counter(target);
            }
            return;
        }

        // Suppress the beggar's alms-request remarks during the ordinary
        // seek + receive-purse chain. `reveal_scrolls` bumps the same counter
        // again at the chain's end, so both sites are needed.
        if command == Command::Pay {
            self.stamp_beggar_dont_talk_counter(target);
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
        let (tgt_pos, tgt_sector, take_tolerance_override, pc_in_coma_carry) = match self
            .get_entity(target)
        {
            Some(e) => {
                let pos_map = e.element_data().position_map();
                let gating_pos = if b_use_action_point {
                    e.cxx_current_point_map().unwrap_or(pos_map)
                } else {
                    pos_map
                };
                (
                    gating_pos,
                    e.element_data().sector(),
                    (command == Command::Take).then(|| take_seek_tolerance(e)),
                    if command == Command::TakeCorpse {
                        match e {
                            Entity::Pc(pc) => self
                                .pc_description_for_pc_data(&pc.pc)
                                .unwrap_or_else(|| {
                                    panic!(
                                        "TakeCorpse target {target:?} is a live PC without its required campaign description"
                                    )
                                })
                                .status
                                .in_coma,
                            _ => false,
                        }
                    } else {
                        false
                    },
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
            // RHElementActorPC::MouseClicked owns a distinct in-coma-PC
            // pickup path. Unlike Human::MouseClicked, it neither casts the
            // lift action distance to UWORD nor uses it unchanged: it keeps
            // the fractional value and adds 10
            // (RHelementactorpc.cpp:1058-1075).
            None if pc_in_coma_carry => {
                match self.actor_action_distance(
                    actor,
                    crate::order::OrderType::TransitionWaitingUprightCarryingCorpse,
                ) {
                    Some(distance) => distance + 10.0,
                    None => return,
                }
            }
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
        //   Heal / Search / SwordstrikeDown / Tie / Take / TakeCorpse →
        //     SEEK_IN_BUILDINGS
        // `NO_TRANSITIONS` suppresses the stand↔crouch retry the seek
        // would otherwise inject; `SEEK_STOP_NPC` asks the victim NPC
        // to halt on arrival; `SEEK_IN_BUILDINGS` lets `RefreshSeek`
        // short-circuit when both actor and target are already inside
        // the same building.
        //
        // TakeCorpse carries the flag from
        // `RHElementActorHuman::MouseClicked`'s carry arm
        // (RHelementactorhuman.cpp:11426-11436). TODO: the PC-specific
        // in-coma pickup (RHelementactorpc.cpp:1058-1095) builds its
        // Seek + post-seek pair by hand and deliberately does NOT pass
        // RHMOVE_SEEK_IN_BUILDINGS; Rust routes every carry click
        // through this one helper, so an in-coma PC target currently
        // gets the flag it should not have.
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
            Command::TakeCorpse if !pc_in_coma_carry => {
                per_command_seek_flags |= MoveFlags::SEEK_IN_BUILDINGS;
            }
            _ => {}
        }

        // Object clicks are a distinct Original path:
        // RHElementObject::MouseClicked always constructs a SEEK element and
        // hangs TAKE off its post-seek sequence, even when the actor is
        // already within `GetRadius() + 15`. The immediately-satisfied seek
        // still has one authoritative frame of lifecycle (MOVE_OK and the
        // seek refresh wait counter) before the TAKE is launched.
        let needs_seek = command == Command::Take || dist > action_distance || !same_sector;
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
                // RHElementActorCivilian::MouseClicked asks
                // AddInteractionWithSeek to face the beggar. The Original
                // translates that boolean into RHMOVE_USE_POINT, so
                // RefreshSeek authorizes the PC's move box at the beggar's
                // live sprite hotspot rather than at its map centre.
                if command == Command::Pay {
                    *flags |= MoveFlags::USE_POINT;
                }
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
            if append_posture_recovery {
                self.append_posture_recovery(actor, &mut post_seek);
            }
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
            // AddInteractionWithSeek builds and launches an RHSequence even
            // when no seek is necessary. Launching the owned element through
            // the eager single-element wrapper would arbitrate immediately,
            // before this frame's actor slot; the Original does not instruct
            // it until SequenceManager::Hourglass at the end of the frame.
            let mut seq = Sequence::new();
            seq.append_element(interaction);
            if append_posture_recovery {
                self.append_posture_recovery(actor, &mut seq);
            }
            self.launch_sequence(seq);
        }
    }

    /// Reproduce `RHElementTarget::MouseClicked`'s ordinary (non-QA) route:
    /// synchronously construct `AppendMoveToSequence(..., victim=target,
    /// tolerance=0)`, then turn to the target hotspot and perform the
    /// resolved interaction.
    fn apply_target_interaction_route(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        actor: EntityId,
        target: EntityId,
        command: Command,
        running: bool,
    ) -> bool {
        let (actor_pos, actor_sector, actor_posture, actor_auth, door, door_direction) = {
            let entity = self
                .get_entity(actor)
                .unwrap_or_else(|| panic!("target interaction requires missing actor {actor:?}"));
            let (door, door_direction) = super::movement::current_door_for_route_source(entity);
            (
                entity.element_data().position_map(),
                super::ai::ai_view_position_sector(self, entity.element_data())
                    .unwrap_or_else(|| panic!("target interaction actor {actor:?} has no sector")),
                entity.element_data().posture,
                entity.actor_auth_info(),
                door,
                door_direction,
            )
        };
        let (target_pos, target_sector, target_layer, target_point) = {
            let entity = self
                .get_entity(target)
                .unwrap_or_else(|| panic!("target interaction requires missing target {target:?}"));
            (
                entity.element_data().position_map(),
                super::ai::ai_view_position_sector(self, entity.element_data()).unwrap_or_else(
                    || panic!("target interaction target {target:?} has no sector"),
                ),
                entity.element_data().layer(),
                entity.cxx_current_point_map().unwrap_or_else(|| {
                    panic!("target interaction target {target:?} has no current point")
                }),
            )
        };

        let action = if running {
            crate::order::OrderType::RunningUpright
        } else if actor_posture == crate::element::Posture::Crouched {
            crate::order::OrderType::WalkingCrouched
        } else {
            crate::order::OrderType::WalkingUpright
        };

        let same_sector = match (actor_sector.arena_index(), target_sector.arena_index()) {
            (Some(actor), Some(target)) => actor == target,
            (None, None) => actor_sector == target_sector,
            (Some(_), None) | (None, Some(_)) => false,
        };
        let (gate_path, gate_source_sector) = if same_sector {
            (Vec::new(), None)
        } else {
            let (source_pos, source_sector) =
                super::movement::adapt_source_to_current_door_with_identity(
                    &self.script_domains.interactables.doors,
                    door,
                    door_direction,
                )
                .map(|(position, sector, _)| (position, sector))
                .unwrap_or((actor_pos, actor_sector));
            let level = self.world.fast_grid.level.clone();
            let Some(path) = crate::gate::find_path_gates_with_sector_indices(
                &self.script_domains.interactables.doors,
                (source_pos.x, source_pos.y),
                source_sector.get(),
                source_sector.arena_index(),
                (target_pos.x, target_pos.y),
                target_sector.get(),
                target_sector.arena_index(),
                Some(&actor_auth),
                false,
                &|sector| self.building_sector_is_authorized(sector),
                &|sector| {
                    level
                        .sectors
                        .iter()
                        .find(|candidate| candidate.sector_number == sector)
                        .and_then(|candidate| candidate.lift_type)
                },
            ) else {
                tracing::warn!(
                    ?actor,
                    ?target,
                    ?command,
                    "RHElementTarget click could not construct its gate route"
                );
                return false;
            };
            (
                path,
                target_interaction_assert_source_sector(source_sector, target_sector),
            )
        };

        let mut turn = SequenceElement::new_generic(1, Command::Turn, Some(actor));
        turn.set_property(
            Field::CameraPoint,
            FieldValue::GeoPoint2D {
                x: target_point.x,
                y: target_point.y,
            },
        );
        let interaction = SequenceElement::new_interaction(2, command, Some(actor), Some(target));

        self.build_gate_movement_sequence(
            sim,
            actor,
            gate_source_sector,
            gate_path,
            GoalShape::Target {
                point: target_pos,
                target,
                tolerance: 0.0,
            },
            target_layer,
            action,
            true,
            1.0,
            MoveFlags::empty(),
            Vec::new(),
            vec![turn, interaction],
            false,
            false,
        )
        .unwrap_or_else(|| {
            panic!("target interaction route for {actor:?} -> {target:?} was empty")
        });
        true
    }

    /// Launch the exact sequence shape authored by the QA branch of
    /// `RHElementTarget::MouseClicked`: a coordinate SEEK at the recorded
    /// target position, with no movement flags and zero tolerance, followed
    /// by the recorded Turn and interaction elements.
    #[allow(clippy::too_many_arguments)]
    fn replay_recorded_target_interaction(
        &mut self,
        actor: EntityId,
        target: EntityId,
        command: Command,
        destination: MapPoint,
        sector: Option<crate::position_interface::SectorHandle>,
        layer: u16,
        action: crate::order::OrderType,
        turn_point: MapPoint,
    ) {
        let mut turn = SequenceElement::new_generic(1, Command::Turn, Some(actor));
        turn.set_property(
            Field::CameraPoint,
            FieldValue::GeoPoint2D {
                x: turn_point.x,
                y: turn_point.y,
            },
        );
        let interaction = SequenceElement::new_interaction(2, command, Some(actor), Some(target));
        let mut post_seek = Sequence::new();
        post_seek.append_element(turn);
        post_seek.append_element(interaction);

        let mut seek = SequenceElement::new_movement(1, Command::Seek, Some(actor), action);
        if let SequenceElementData::Movement {
            destination: seek_destination,
            sector: seek_sector,
            layer: seek_layer,
            element,
            tolerance,
            flags,
            post_seek_sequence,
            ..
        } = &mut seek.data
        {
            *seek_destination = destination;
            *seek_sector = sector;
            *seek_layer = layer;
            *element = None;
            *tolerance = 0.0;
            *flags = MoveFlags::empty();
            *post_seek_sequence = Some(Box::new(post_seek));
        }

        let mut sequence = Sequence::new();
        sequence.append_element(seek);
        self.launch_sequence(sequence);
    }

    /// Fire `EVENT_STOP` on a target NPC that a PC is currently
    /// seeking with `SEEK_STOP_NPC`.  No-op when the target isn't an
    /// NPC or isn't in a moving action state.
    pub(crate) fn send_seek_stop_to_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        target: EntityId,
    ) {
        {
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
            let Some(_base) = npc.ai_brain.base_mut() else {
                return;
            };
        }

        // C++ calls `target->Think(EVENT_STOP)` directly from RefreshSeek.
        // Use the canonical synchronous Think boundary so the causal stop
        // runs now while older deferred detection stimuli retain their FIFO.
        // Delaying EVENT_STOP to the end-of-frame self-stimulus drain lets a
        // registered gate successor enter non-interruptible PassDoor first.
        self.dispatch_synchronous_ai_think_preserving_detection_fifo(
            sim,
            target,
            assets,
            crate::ai::Stimulus::new(crate::ai::StimulusType::EventStop),
        );
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
    fn apply_scroll_read_with_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        actor: EntityId,
        target: EntityId,
        running: bool,
    ) {
        use crate::sequence::{Field, FieldValue};

        // `running && !is_recording` is a short-circuit — the PC just
        // gets `MakeFast` and we never build the composite.
        let is_recording = self.is_recording_macro();
        if running && !is_recording {
            self.actor_make_fast(sim, actor);
            return;
        }

        let (pc_pos, pc_posture) = match self.get_entity(actor) {
            Some(e) => (e.element_data().position_map(), e.element_data().posture),
            None => return,
        };
        let (npc_pos, attached_scroll, npc_ai_script_locked) = match self.get_entity(target) {
            Some(e) => {
                let attached_scroll = match e {
                    crate::element::Entity::Soldier(s) => s.npc.attached_scroll,
                    crate::element::Entity::Civilian(c) => c.npc.attached_scroll,
                    _ => None,
                };
                let locked = e.ai_controller().is_some_and(|ai| ai.ai_is_script_locked());
                (e.element_data().position_map(), attached_scroll, locked)
            }
            None => return,
        };
        let Some(scroll_id) = attached_scroll else {
            tracing::warn!(
                ?actor,
                ?target,
                "apply_scroll_read_with_seek: target NPC is not scroll-attached"
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
                self.apply_disable_all_actions_temp(0, Some(actor));
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

        // NPC::MouseClicked passes the literal distance 30 to
        // `AddSequenceWithSeek`; it is intentionally not derived from an
        // animation profile (the Original even marks that constant FIXME).
        let action_distance = 30.0;

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

        // RHElementActorPC::MouseClicked authors this point seek with the
        // literal tolerance 8.f.  This interaction does not use the sprite
        // action point distance used by the generic interaction helper.
        let action_distance = 8.0;

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
        sim: &crate::sim_rng::SimulationContext,
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
                let mut sequence = Sequence::new();
                sequence.append_element(speak);
                self.launch_sequence(sequence);
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
                Entity::Soldier(s) => (
                    s.camp() == Camp::Lacklandists,
                    s.npc.attached_scroll.is_some(),
                ),
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
                self.apply_interaction_with_seek(sim, pc_id, launch_target, cmd, false);
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
            &self.world.entities,
            &self.world.fast_grid,
            &assets.profile_manager,
            pc_id,
            target_id,
        ) {
            self.apply_table_swordfight(pc_id, target_id, aggressor_line_idx, action_style);
            return;
        }

        // Classical seek + enter
        let pc_profile_index = self
            .get_entity(pc_id)
            .and_then(|entity| entity.pc_data())
            .map(|pc| pc.profile_index);
        let hth_weapon_id = self.get_entity(pc_id).and_then(|entity| {
            crate::engine::melee::get_hth_weapon_id_full(entity, &assets.profile_manager)
        });
        let seek_tolerance = hth_weapon_id
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .map(|p| p.distance[crate::weapons::WeaponDistance::Default as usize] as f32)
            .unwrap_or(40.0);
        tracing::trace!(
            actor = ?pc_id,
            target = ?target_id,
            ?pc_profile_index,
            ?hth_weapon_id,
            ?action_style,
            seek_distance = seek_tolerance,
            "creating classical swordfight entity seek"
        );

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
                .map(crate::engine::movement::current_door_for_route_source)
                .unwrap_or((crate::position_interface::DoorHandle::NULL, false));
            let (adj_src_pos, adj_src_sector) = {
                let adapted = crate::engine::movement::adapt_source_to_current_door(
                    &self.script_domains.interactables.doors,
                    door_handle,
                    door_direction,
                );
                match adapted {
                    Some((adj, sector, _layer)) => (adj, sector),
                    None => (MapPoint::new(pc_pos.x, pc_pos.y), u16::from(pcs)),
                }
            };
            // PC authorisation for the gate A*.  Seek/melee routing
            // never sets the leave-map flag, so `allow_leave_map = false`.
            let pc_auth = self
                .get_entity(pc_id)
                .expect("swordfight routing PC disappeared after source snapshot")
                .actor_auth_info();
            let level = self.world.fast_grid.level.clone();
            let gate_path = crate::gate::find_path_gates(
                &self.script_domains.interactables.doors,
                (adj_src_pos.x, adj_src_pos.y),
                adj_src_sector,
                (target_pos.x, target_pos.y),
                ts.into(),
                Some(&pc_auth),
                false,
                &|sector| self.building_sector_is_authorized(sector),
                &|sector| {
                    level
                        .sectors
                        .iter()
                        .find(|candidate| candidate.sector_number == sector)
                        .and_then(|candidate| candidate.lift_type)
                },
            );
            // Detect a swordfight-line pair between the PC's sector
            // and the target's sector — the "across gates" snap case.
            // Computed regardless of whether `find_path_gates`
            // succeeded so the fallback branches below can also use a
            // line-arrival on it.
            let swordfight_line = crate::engine::melee::table_swordfight_jump_line(
                &self.world.fast_grid,
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
                && swordfight_line_idx.is_some()
            {
                let (goal_shape, arrival_layer) = if let Some(aggr_idx) = swordfight_line_idx
                    && let Some(jl) = self
                        .world
                        .fast_grid
                        .level
                        .jump_lines
                        .get(usize::from(aggr_idx))
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
                        GoalShape::Point {
                            point: MapPoint::new(target_pos.x, target_pos.y),
                            tolerance: seek_tolerance,
                        },
                        target_layer,
                    )
                };

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

                // This branch is only the across-jump-line snap case. An
                // ordinary cross-gate swordfight falls through to the real
                // entity SEEK below, whose Translate -> RefreshSeek lowering
                // stamps TIME_SEEK_REFRESH, passes the victim to every gate
                // approach, and retains ENTER_SWORDFIGHT as actor-owned
                // post-seek work exactly like the Original. Arrival speech
                // and generic posture recovery belong to PC group moves, not
                // RHElementActorSoldier::MouseClicked's interaction.
                let _ = self.build_gate_movement_sequence(
                    sim,
                    pc_id,
                    Some(
                        crate::position_interface::SectorHandle::new(adj_src_sector)
                            .unwrap_or_else(|| {
                                panic!(
                                    "swordfight route for {pc_id:?} adapted to invalid source sector {adj_src_sector}"
                                )
                            }),
                    ),
                    path,
                    goal_shape,
                    arrival_layer,
                    action_style,
                    true,
                    1.0,
                    MoveFlags::empty(),
                    Vec::new(),
                    vec![enter_elem],
                    false,
                    false,
                );
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
                && let Some(jl) = self
                    .world
                    .fast_grid
                    .level
                    .jump_lines
                    .get(usize::from(aggr_idx))
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
            .world
            .fast_grid
            .level
            .jump_lines
            .get(aggressor_line_idx as usize)
        {
            Some(l) => (l.clone(), l.associated_line_index),
            None => return,
        };
        let Some(victim_line) = victim_line_idx.and_then(|idx| {
            self.world
                .fast_grid
                .level
                .jump_lines
                .get(idx as usize)
                .cloned()
        }) else {
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
        resolved_seek_distance: Option<f32>,
    ) {
        use crate::order::OrderType;

        let same_sector = match (self.get_entity(pc_id), self.get_entity(target_id)) {
            (Some(pc), Some(target)) => {
                pc.element_data().sector() == target.element_data().sector()
            }
            _ => false,
        };

        let strike_elem = SequenceElement::new_interaction(
            if same_sector { 2 } else { 1 },
            strike_cmd,
            Some(pc_id),
            Some(target_id),
        );

        if !same_sector {
            let mut sequence = Sequence::new();
            sequence.append_element(strike_elem);
            self.launch_sequence(sequence);
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
            let mut sequence = Sequence::new();
            sequence.append_element(strike_elem);
            self.launch_sequence(sequence);
            return;
        };

        let target_distance = if let Some(distance) = resolved_seek_distance {
            assert!(
                distance.is_finite() && distance >= 0.0,
                "resolved sword seek distance must be finite and non-negative"
            );
            distance
        } else {
            let Some(distance) = self
                .get_entity(pc_id)
                .and_then(|e| {
                    crate::engine::melee::get_hth_weapon_id_full(e, &assets.profile_manager)
                })
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                .map(|p| legacy_sword_seek_distance(p, strike_cmd, strike))
            else {
                tracing::warn!(
                    ?pc_id,
                    ?target_id,
                    ?strike_cmd,
                    "apply_sword_strike_with_seek: actor has no hth weapon profile; launching direct strike"
                );
                let mut sequence = Sequence::new();
                sequence.append_element(strike_elem);
                self.launch_sequence(sequence);
                return;
            };
            distance
        };

        // RHEngine::PerformSwordfight authors this seek as
        // RHNONANIMATION_RUNNING_WITH_SWORD.  FORCE_SWORD_MOVEMENT is a
        // separate policy bit and must remain clear: if the opponent goes
        // away before Execute, Human's ordinary orphan-sword guard still
        // aborts the movement and quits swordfight.
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
            *flags |= MoveFlags::SEEK;
            *post_seek_sequence = Some(Box::new(post_seek));
        }

        let mut sequence = Sequence::new();
        sequence.append_element(seek_elem);
        // LaunchSequenceElement registers this seek at the sequence manager's
        // tail. It does not arbitrate it synchronously against an older
        // postponed chain: if that chain is released before Hourglass reaches
        // this new seek, the older successor is instructed first.
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
        already_authorized: bool,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
        recorded_gate_path: Option<crate::gate::RecordedGatePath>,
    ) {
        self.apply_drop_ale_at_with_recovery(
            actor,
            target_pos,
            running,
            false,
            already_authorized,
            goal_override,
            goal_sector_index_override,
            recorded_gate_path,
        );
    }

    /// Build the ordinary DropAle route, optionally retaining quick-action
    /// posture recovery in its post-seek sequence.
    fn apply_drop_ale_at_with_recovery(
        &mut self,
        actor: EntityId,
        target_pos: crate::coordinates::MapPoint,
        running: bool,
        append_posture_recovery: bool,
        already_authorized: bool,
        goal_override: Option<(crate::sector::SectorNumber, u16)>,
        goal_sector_index_override: Option<crate::fast_find_grid::SectorIndex>,
        recorded_gate_path: Option<crate::gate::RecordedGatePath>,
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

        // The drop point's sector and layer come from the cursor, not from
        // the actor: take the topmost sector under the point, resolve a patch
        // overlay to the sector it covers, and resolve a jump sector to the
        // sector it sits in. Jump sectors carry no sector number of their own,
        // so reading the number off the raw hit loses the goal entirely and
        // the seek never learns that it has to cross a gate.
        //
        // TODO: the original also refuses the whole action when the resolved
        // sector is a door, or a lift that is a wall or ladder. Not ported
        // yet — a mis-resolution here would silently drop a replayed command,
        // so the guard needs its own validation pass.
        assert_eq!(
            already_authorized,
            goal_override.is_some(),
            "resolved DropAle commands must carry both already_authorized and goal_override"
        );
        assert!(
            goal_sector_index_override.is_none() || goal_override.is_some(),
            "DropAle exact goal-sector identity requires a goal_override"
        );
        let (goal_sector, goal_layer) = if let Some((goal_sector, goal_layer)) = goal_override {
            let public_sector = u16::from(goal_sector);
            let mut sector = crate::position_interface::SectorHandle::new(public_sector)
                .unwrap_or_else(|| {
                    panic!("DropAle goal_override has invalid public sector {public_sector}")
                });
            if let Some(index) = goal_sector_index_override {
                let indexed_sector = self
                    .world
                    .fast_grid
                    .level
                    .sectors
                    .get(usize::from(index))
                    .unwrap_or_else(|| {
                        panic!(
                            "DropAle exact goal-sector arena {index} is outside the FastFindGrid sector table"
                        )
                    });
                assert_eq!(
                    indexed_sector.sector_number, goal_sector,
                    "DropAle exact goal-sector arena {index} has public sector {}, expected {goal_sector}",
                    indexed_sector.sector_number,
                );
                sector = sector.with_arena_index(index);
            }
            (Some(sector), goal_layer)
        } else {
            let reference = self
                .get_entity(actor)
                .map(|e| e.element_data().position_map())
                .unwrap_or(target_pos);
            let hit = self
                .world
                .fast_grid
                .get_sector_screen(target_pos, reference);
            let resolved = hit.sector_idx.map(|idx| {
                let sector = self
                    .world
                    .fast_grid
                    .level
                    .sectors
                    .get(usize::from(idx))
                    .unwrap_or_else(|| panic!("DropAle sector hit references missing arena {idx}"));
                if sector.sector_type.is_patch() || sector.sector_type.is_jump() {
                    // Original immediately dereferences RHSectorJump::GetSector
                    // here (`RHEngine::ManageInputActionAle`); an overlay
                    // without its authored underlying sector is corrupt
                    // topology, not a number-only compatibility case.
                    let under_idx = sector.underlying_sector.unwrap_or_else(|| {
                        panic!("DropAle overlay sector arena {idx} has no underlying sector")
                    });
                    let under = self
                        .world
                        .fast_grid
                        .level
                        .sectors
                        .get(usize::from(under_idx))
                        .unwrap_or_else(|| {
                            panic!(
                                "DropAle overlay sector arena {idx} references missing underlying arena {under_idx}"
                            )
                        });
                    let handle = crate::position_interface::SectorHandle::new(u16::from(
                        under.sector_number,
                    ))
                    .unwrap_or_else(|| {
                        panic!(
                            "DropAle overlay sector arena {idx} resolved invalid underlying public sector {}",
                            under.sector_number
                        )
                    })
                    .with_arena_index(under_idx);
                    (handle, sector.layer)
                } else {
                    (
                        hit.sector_handle().unwrap_or_else(|| {
                            panic!("DropAle valid sector hit arena {idx} has no public handle")
                        }),
                        hit.layer,
                    )
                }
            });
            match resolved {
                Some((sector, layer)) => (Some(sector), layer),
                None => (hit.sector_handle(), hit.layer),
            }
        };

        // The move box is authorised on the cursor's layer, the same one the
        // seek element is stamped with.
        let mut destination_pos = target_pos;
        if !already_authorized && move_box.is_somewhere() {
            let mut box_at_target = move_box.translated(target_pos);
            if self
                .world
                .fast_grid
                .find_authorized_position(&mut box_at_target, goal_layer)
            {
                let center = box_at_target.center();
                destination_pos = crate::coordinates::MapPoint {
                    x: center.x,
                    y: center.y,
                };
            } else {
                tracing::warn!(
                    ?actor,
                    goal_layer,
                    target_x = target_pos.x,
                    target_y = target_pos.y,
                    "apply_drop_ale_at: target move box has no authorized position"
                );
                return;
            }
        }

        tracing::trace!(
            ?actor,
            actor_sector = ?self.get_entity(actor).and_then(|e| e.element_data().sector()),
            actor_layer = layer,
            ?goal_sector,
            goal_layer,
            target_x = target_pos.x,
            target_y = target_pos.y,
            dest_x = destination_pos.x,
            dest_y = destination_pos.y,
            "apply_drop_ale_at: resolved drop goal"
        );

        let mut move_elem =
            SequenceElement::new_movement(1, Command::Seek, Some(actor), action_style);
        move_elem.recorded_gate_path = recorded_gate_path;
        if let SequenceElementData::Movement {
            destination,
            tolerance,
            flags,
            post_seek_sequence,
            sector,
            layer: elem_layer,
            ..
        } = &mut move_elem.data
        {
            *destination = destination_pos;
            *tolerance = action_distance;
            *flags |= MoveFlags::SEEK;
            *sector = goal_sector;
            *elem_layer = goal_layer;
            let mut post_seek = Sequence::new();
            post_seek.append_element(SequenceElement::new(1, Command::DropAle, Some(actor)));
            if append_posture_recovery {
                self.append_posture_recovery(actor, &mut post_seek);
            }
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
        danger_point: crate::coordinates::WorldPoint3D,
        danger_point_layer: u16,
    ) {
        use crate::order::OrderType;

        self.world.shield.is_protected = true;
        self.world.shield.protected_pc = Some(protected_pc);
        self.world.shield.danger_point = danger_point;
        self.world.shield.danger_point_layer = danger_point_layer;

        // Stamp the new danger point on the acting PC so
        // `sync_danger_point_titbits` refreshes the `DangerPoint`
        // titbit next tick.
        if let Some(entity) = self.world.entities.get_mut(actor)
            && let Some(actor_data) = entity.actor_data_mut()
        {
            actor_data.shield_face_point = Some(danger_point.to_map());
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
                z: danger_point.z,
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
            && let Some(me) = self.world.entities.get_mut(protector_id)
            && let Some(pc) = me.pc_data_mut()
        {
            pc.shield_danger_point = crate::coordinates::WorldPoint3D::default();
        }

        if let Some(me) = self.world.entities.get_mut(protector_id)
            && let Some(pc) = me.pc_data_mut()
        {
            pc.shield_protected = protectee;
        }
    }

    fn apply_crouch_down(&mut self, sim: &crate::sim_rng::SimulationContext, seat: usize) {
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
            self.actor_make_crouched(sim, pc_id);
        }
        if recorded_here {
            self.stop_recording_macro();
        }
    }

    fn apply_stand_up(&mut self, sim: &crate::sim_rng::SimulationContext, seat: usize) {
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
                    self.actor_make_upright(sim, pc_id);
                }
                crate::element::Posture::SimulatingBeggar => {
                    let elem = SequenceElement::new(1, Command::LeaveBeggar, Some(pc_id));
                    let mut sequence = Sequence::new();
                    sequence.append_element(elem);
                    self.launch_sequence(sequence);
                }
                crate::element::Posture::Spy | crate::element::Posture::AnonymousArcher => {
                    let elem = SequenceElement::new(1, Command::LeaveSpy, Some(pc_id));
                    let mut sequence = Sequence::new();
                    sequence.append_element(elem);
                    self.launch_sequence(sequence);
                }
                crate::element::Posture::Tree => {
                    let elem = SequenceElement::new(1, Command::LeaveTree, Some(pc_id));
                    let mut sequence = Sequence::new();
                    sequence.append_element(elem);
                    self.launch_sequence(sequence);
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
        && let Some(campaign) = Some(&engine.mission_domain.campaign)
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

    let difficulty = engine.control.sim_config.difficulty;

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
                && c.npc.attached_scroll.is_none())
    {
        let ransom = Some(&engine.mission_domain.campaign)
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
        // Original SwordstrikeDown passes the literal `40` to
        // AddInteractionWithSeek (RHelementactornpc.cpp).
        Command::SwordstrikeDown => 40.0,
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
            "a genuinely distinct adapted source must retain AppendMoveToSequence's leading AssertPosition"
        );

        let source_index = crate::fast_find_grid::SectorIndex::new(17).unwrap();
        let target_index = crate::fast_find_grid::SectorIndex::new(18).unwrap();
        let exact_source = target_sector.with_arena_index(source_index);
        let exact_target = target_sector.with_arena_index(target_index);
        assert_eq!(
            target_interaction_assert_source_sector(exact_source, exact_target),
            Some(exact_source),
            "overlapping public sector numbers are distinct Original RHSector pointers"
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
            "loaded actors that retained only a public number recover Original's exact sector pointer"
        );
        assert_eq!(
            target_interaction_assert_source_sector(recovered_source, exact_target),
            None,
            "same-sector target interactions must emit Move directly instead of AssertPosition then losing Move at the building tail"
        );
    }

    /// A legacy `SwordStrikeCmd` carries no resolved seek tolerance, so
    /// the distance has to be rebuilt from the command. Thrust A is the
    /// command `RHEngine::PerformSwordfight` hard-codes on its
    /// no-gesture click arm, where `ConvertMousePatternToStrike` yields
    /// `END_OF_REAL_STRIKE` and `RHSword::GetStrikeMaximalDistance`
    /// answers with the weapon's generic maximum. Thrust B..E can only
    /// come from the gesture arm and keep their per-thrust maximum.
    #[test]
    fn legacy_sword_seek_distance_uses_generic_maximum_only_for_thrust_a() {
        let mut weapon = crate::profiles::HtHWeaponProfile::default();
        weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] = 70;
        weapon.thrusts[crate::weapons::SwordStrike::A as usize].maximal_distance = 60;
        weapon.thrusts[crate::weapons::SwordStrike::B as usize].maximal_distance = 80;

        assert_eq!(
            legacy_sword_seek_distance(
                &weapon,
                Command::SwordstrikeThrustA,
                crate::weapons::SwordStrike::A,
            ),
            63.0
        );
        assert_eq!(
            legacy_sword_seek_distance(
                &weapon,
                Command::SwordstrikeThrustB,
                crate::weapons::SwordStrike::B,
            ),
            72.0
        );
    }

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
        engine.mission_domain.campaign = campaign;

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
                campaign_description_index: Some(0),
                life_points: 50,
                ..PcData::default()
            },
        }));

        (engine, assets, pc_id)
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
        assert_eq!(titbit.element_supplier, ElementHandle(protected_pc.index()));
        assert_eq!(titbit.element_manager, ElementHandle(actor.index()));
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
            .as_deref()
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
                .all(|titbit| titbit.id != original_titbit.get()),
            "the first replacement append must retire the former titbit atomically"
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
            },
        };
        let manual = engine.players.macro_store.get_or_insert(pc_id);
        manual.begin_recording(0);
        manual.append_if_recording(manual_step.clone());
        manual.stop_recording();
        let queued = PlayerCommand::QueueQuickAction {
            action: Action::Whistle,
            command: Box::new(PlayerCommand::LaunchSelfAbility {
                actor: pc_id,
                command: Command::WhistleCmd,
            }),
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
        engine.advance_auto_quick_action_queues(&sim, &mut display, &assets);

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
            command: Box::new(PlayerCommand::LaunchSelfAbility {
                actor: pc_id,
                command: Command::WhistleCmd,
            }),
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
    fn queued_bow_shot_starts_after_real_work_ends_despite_postponed_card() {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[(Action::Bow, 1)]);
        let target = spawn_pc_at(&mut engine, 90.0, 10.0);
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
                command: Box::new(PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target,
                    command: Command::ShootBow,
                    running: false,
                }),
            },
        );
        assert!(engine.has_quick_action(pc_id, 0));

        engine
            .orders
            .sequence_manager
            .element_terminated(busy_sequence, 0);
        engine.advance_auto_quick_action_queues(&sim, &mut display, &assets);

        assert!(!engine.has_quick_action(pc_id, 0));
        assert!(
            engine
                .orders
                .sequence_manager
                .has_live_element_for_actor_matching(pc_id, |command| {
                    command == Command::ShootBow
                }),
            "the queued bow interaction must launch when only dormant work remains"
        );
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
                command: Box::new(PlayerCommand::LaunchInteraction {
                    actor: pc_id,
                    target,
                    command: Command::Take,
                    running: false,
                }),
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
            crate::titbit::QuickAction::Take as u16
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

        // Execute/PerformAction has already selected row 1. The popup's
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
        // PerformAction selected base + old direction before nested Refresh.
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
                // Deliberately independent identities: neither the first
                // matching profile nor mubListIndex owns this actor's status.
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

    fn setup_take_corpse_macro_scene(
        target_x: f32,
    ) -> (EngineInner, LevelAssets, EntityId, EntityId) {
        let (mut engine, assets, pc_id) = setup_pc_engine(&[]);
        let sector = crate::position_interface::SectorHandle::new(1);
        {
            let pc = engine.get_entity_mut(pc_id).expect("test PC exists");
            pc.element_data_mut().posture = Posture::HelpingToClimb;
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
            element: ElementData {
                kind: ElementKind::ActorPc,
                active: true,
                posture: Posture::Lying,
                ..ElementData::default()
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
            "Original StartQuickAction appends posture recovery to the recorded sequence"
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
            .as_deref()
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
            pc.element_data_mut().posture = Posture::HelpingToClimb;
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
            .as_deref()
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
            // sector number was the same. Original compares the pointers.
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
    }

    #[test]
    fn resolved_replay_drop_ale_preserves_authorized_point_and_route_goal() {
        let (mut engine, assets, pc_id, source_index, goal_index) =
            setup_drop_ale_sector_identity_scene();
        let authorized = crate::coordinates::MapPoint::new(2607.467_041, 881.610_474);
        let recorded_gate_path = crate::gate::RecordedGatePath {
            source_sector: crate::sector::SectorNumber::new(0),
            source_sector_index: Some(source_index),
            source_layer: 0,
            outcome: crate::gate::RecordedGateOutcome::Success(vec![crate::gate::GatePathStep {
                door_index: crate::gate::DoorIndex(42),
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
            pc.position_iface_mut()
                .set_door(crate::position_interface::DoorHandle(7), true);
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
            pc.position_iface_mut()
                .set_door(crate::position_interface::DoorHandle(7), false);
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
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
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
            .as_deref()
            .expect("live Strangle seek retains its interaction");
        assert_eq!(post_seek.len(), 1);
        assert_eq!(post_seek.get(0).unwrap().command, Command::StrangleCmd);
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

    fn spawn_friendly_civilian(engine: &mut EngineInner) -> EntityId {
        let mut civilian = ActorCivilian {
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
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
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData::default(),
            civilian: Default::default(),
        };
        target.element.set_sector(sector);
        let target_id = engine.add_entity(Entity::Civilian(target));

        let mut legacy = engine.clone();
        legacy.apply_sword_strike_with_seek(
            &assets,
            pc_id,
            target_id,
            Command::SwordstrikeThrustD,
            None,
        );
        assert_eq!(
            first_seek_tolerance(&legacy),
            54.0,
            "missing resolved distance must preserve legacy strike-specific lookup"
        );

        engine.apply_sword_strike_with_seek(
            &assets,
            pc_id,
            target_id,
            Command::SwordstrikeThrustD,
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
            .as_deref()
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
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
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
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
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
        let mut display = HostDisplayState::default();
        engine.hourglass_phase_sequences(&sim, &mut display, &assets);

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
        assert!(
            !commands
                .iter()
                .any(|command| *command == Command::SpeakHeroReachDestination)
        );
        assert!(!commands.iter().any(|command| *command == Command::EquipBow));

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
            .as_deref()
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
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
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
        );

        // LaunchSequenceElement admission: the newer seek is registered at
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
            "the newer seek waits for Hourglass instead of replacing the postponed chain"
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
            element: ElementData {
                kind: ElementKind::ActorCivilian,
                active: true,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                attached_scroll: None,
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
            target.element_data_mut().posture = Posture::Lying;
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
            "the PC-specific carry branch does not pass Human::MouseClicked's building flag"
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
            target.element_data_mut().posture = Posture::Lying;
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
                element: ElementData {
                    kind: ElementKind::Target,
                    active: true,
                    ..ElementData::default()
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

            engine.hourglass_phase_sequences(&sim, &mut display, &assets);
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
            pc.element_data_mut().posture = Posture::Crouched;
        }

        let mut target = ElementTarget {
            element: ElementData {
                kind: ElementKind::Target,
                active: true,
                ..ElementData::default()
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
            .and_then(Entity::cxx_current_point_map)
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
            .as_deref()
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

        engine.hourglass_phase_sequences(&sim, &mut display, &assets);
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
        // Original RHCOMMAND_PAY is created only by
        // RHElementActorCivilian::MouseClicked after `IsBeggar()` succeeds;
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
        // C++ PCs read ammo from their own mpStatus, not from a
        // campaign array slot equal to the first matching profile or
        // mubListIndex.
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
        if let Some(campaign) = Some(&mut engine.mission_domain.campaign)
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
        // host resolves it after RecordFrame, so its trace event appears on
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

        engine.hourglass_phase_sound_boundary(sim, &assets);
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
        engine.hourglass_phase_sound_boundary(sim, &assets);

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
}
