//! Quick-action capture, queue advancement, and replay execution.
//!
//! The parent dispatcher alone decides when shared recording runs. Nested replay
//! enters that same dispatcher; it must not bypass preflight or batch semantics.

use super::{
    determine_use_command, recorded_ground_target_titbit_layer, recorded_interaction_quick_phase,
};
use crate::coordinates::MapPoint;
use crate::element::{Command, Entity, EntityId};
use crate::engine::movement::PlannedRecordedGroupMoveOutcome;
use crate::engine::{CameraDisplayState, EngineInner, LevelAssets};
use crate::player_command::PlayerCommand;
use crate::profiles::Action;
use crate::sequence::{Sequence, SequenceElement};
use crate::titbit::{ElementHandle, INVALID_ID, QuickAction, TitbitKind};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum QuickActionRecordingStore {
    Manual,
    Automatic,
}

#[inline]
/// Original-game posture recovery rewrites a terminal
/// `SHOOT_BOW` to `SHOOT_BOW_ONCE` when the actor was not already aiming.
pub(super) fn quick_action_tail_command(
    command: Command,
    is_tail: bool,
    was_aiming: bool,
) -> Command {
    if is_tail && command == Command::ShootBow && !was_aiming {
        Command::ShootBowOnce
    } else {
        command
    }
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

impl EngineInner {
    /// Append a `QuickActionStep` to the currently-recording PC's
    /// macro, if a recording is in progress and the command targets
    /// that PC.  No-op otherwise.
    ///
    /// Only the `Action` + target `position` is stored per step — the
    /// per-slot titbit id is set separately when the titbit is added.
    pub(super) fn record_macro_step_for(
        &mut self,
        seat: usize,
        cmd: &PlayerCommand,
        assets: &LevelAssets,
    ) {
        if self.players.qa_recording_for.is_empty() {
            return;
        }
        // The original game executes recognized swordfight gestures
        // gestures directly, without consulting macro-recording status or
        // installing a quick action.
        // Entering a swordfight through the soldier click path is recordable,
        // but an attack made once the fight is active is not. Keep the lower
        // per-PC recorder arm for the explicit `QueueQuickAction` extension,
        // which calls it directly rather than passing through this manual
        // recording hook.
        if matches!(cmd, PlayerCommand::SwordStrikeCmd { .. }) {
            return;
        }
        // GroupMove recording belongs inside group movement's per-PC
        // formation/authorization pass. Recording the common click here loses
        // the adjusted slot and exact route goal that Original retains.
        if matches!(cmd, PlayerCommand::GroupMove { .. }) {
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
                None,
                assets,
                QuickActionRecordingStore::Manual,
            );
        }
    }

    pub(in crate::engine) fn record_resolved_group_move_step(
        &mut self,
        recording_pc: EntityId,
        destination: MapPoint,
        running: bool,
        route: crate::macro_store::RecordedQaMoveRoute,
        assets: &LevelAssets,
    ) {
        self.record_resolved_group_move_step_in_store(
            recording_pc,
            destination,
            running,
            route,
            None,
            assets,
            QuickActionRecordingStore::Manual,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_resolved_group_move_step_in_store(
        &mut self,
        recording_pc: EntityId,
        destination: MapPoint,
        running: bool,
        route: crate::macro_store::RecordedQaMoveRoute,
        action_override: Option<crate::profiles::Action>,
        assets: &LevelAssets,
        recording_store: QuickActionRecordingStore,
    ) {
        let command = PlayerCommand::GroupMove {
            actors: vec![recording_pc],
            destination,
            running,
            show_marker: false,
            goal_override: None,
            goal_sector_index_override: None,
            door_route_override: None,
            recorded_gate_routes: Vec::new(),
            recorded_failed_gate_routes: Vec::new(),
        };
        self.record_macro_step_for_pc(
            0,
            &command,
            recording_pc,
            action_override,
            Some(route),
            assets,
            recording_store,
        );
    }

    pub(super) fn record_macro_step_for_pc(
        &mut self,
        seat: usize,
        cmd: &PlayerCommand,
        recording_pc: EntityId,
        action_override: Option<crate::profiles::Action>,
        resolved_move_route: Option<crate::macro_store::RecordedQaMoveRoute>,
        assets: &LevelAssets,
        recording_store: QuickActionRecordingStore,
    ) {
        use crate::macro_store::QuickActionStep;
        use PlayerCommand::*;

        // These commands finish recording in their dispatch arms, where all
        // of the original game's hint arguments have already been resolved.
        // The shared recorder still owns the semantic macro step, but must
        // not allocate a provisional titbit that the dispatch arm immediately
        // removes and replaces. Besides being dead work, that advances the
        // serialized TitbitManager::current_id twice for one Original titbit.
        // Auto-queue recording supplies `action_override` and calls this
        // helper without dispatching the nested command, so it must still
        // allocate here.
        let dispatch_arm_records_titbit = action_override.is_none()
            && matches!(
                cmd,
                LaunchInteraction { .. } | LaunchGroundTarget { .. } | LaunchScrollRead { .. }
            );

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
                        route: resolved_move_route.unwrap_or_else(|| {
                            panic!(
                                "group-move quick action for {recording_pc:?} has no resolved route"
                            )
                        }),
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
                let replay =
                    if matches!(target_entity, crate::element::Entity::Target(_))
                        && matches!(
                            command,
                            Command::SearchCmd
                                | Command::UseLever
                                | Command::HitTarget
                                | Command::HandleTarget
                                | Command::TakeTarget
                                | Command::Pay
                        )
                    {
                        let movement_action = if *running {
                            crate::order::OrderType::RunningUpright
                        } else if self.get_entity(*actor).is_some_and(|entity| {
                            entity.element_data().posture() == crate::element::Posture::Crouched
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
                        turn_point: target_entity.current_gameplay_point_map().unwrap_or_else(|| {
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
                already_authorized,
                goal_override,
                goal_sector_index_override,
                recorded_gate_path,
            } => {
                if *actor != recording_pc {
                    return;
                }
                running_move = *running;
                // Ale-input processing authorizes the actor's translated
                // move box and constructs the concrete Seek before handing
                // that sequence to quick-action assignment. A live input
                // command still carries the raw cursor here, so resolve it at
                // recording time and retain the exact sparse goal identity.
                // Re-authorizing or spatially re-querying during playback can
                // select a different point/floor in overlapping geometry.
                let Some((destination, goal_sector, goal_layer)) = self.resolve_drop_ale_target(
                    *actor,
                    *target_pos,
                    *already_authorized,
                    *goal_override,
                    *goal_sector_index_override,
                ) else {
                    return;
                };
                let goal_sector = goal_sector.unwrap_or_else(|| {
                    panic!(
                        "recorded DropAle target ({}, {}) has no authoritative sector",
                        target_pos.x, target_pos.y
                    )
                });
                let resolved_goal = Some((
                    crate::sector::SectorNumber::new(
                        i16::try_from(u16::from(goal_sector)).unwrap_or_else(|_| {
                            panic!("DropAle goal sector {goal_sector:?} exceeds i16")
                        }),
                    ),
                    goal_layer,
                ));
                (
                    *actor,
                    crate::profiles::Action::Ale,
                    // The original game's recording hint stays at the click projection;
                    // only the stored Seek uses the authorized box center.
                    *target_pos,
                    QaReplayCommand::DropAle {
                        target_pos: destination,
                        running: *running,
                        already_authorized: true,
                        goal_override: resolved_goal,
                        goal_sector_index_override: goal_sector.arena_index(),
                        recorded_gate_path: recorded_gate_path.clone(),
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
                composite,
                gesture_quality,
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
                        composite: *composite,
                        gesture_quality: *gesture_quality,
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
            replay: replay.clone(),
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

        if dispatch_arm_records_titbit {
            return;
        }

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
        let phase = match (phase_override, &replay) {
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
                let target_entity = self.get_entity(*target).unwrap_or_else(|| {
                    panic!("quick-action interaction target {target:?} disappeared")
                });
                if let Some(phase) = recorded_interaction_quick_phase(*command) {
                    phase as u16
                } else if *command == Command::Take
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
                    crate::engine::target_interaction::target_qa_titbit(
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
        let supplier = match &replay {
            QaReplayCommand::Interaction { target, .. }
            | QaReplayCommand::TargetInteraction { target, .. }
            | QaReplayCommand::ScrollRead { target, .. }
            | QaReplayCommand::Swordfight { target, .. }
            | QaReplayCommand::SwordStrike { target, .. }
            | QaReplayCommand::ShieldRaise {
                protected_pc: target,
                ..
            } => Some(*target),
            QaReplayCommand::SelfAbility { .. } | QaReplayCommand::PostureToggle { .. } => {
                Some(actor)
            }
            QaReplayCommand::Move { .. }
            | QaReplayCommand::TacticalMove { .. }
            | QaReplayCommand::GroundTarget { .. }
            | QaReplayCommand::DropAle { .. } => None,
        };
        let actor_layer = self
            .get_entity(recording_pc)
            .map(|entity| entity.element_data().layer())
            .unwrap_or_else(|| panic!("quick-action recording PC {recording_pc:?} disappeared"));
        let (pos3d, layer) = match &replay {
            QaReplayCommand::GroundTarget {
                target_pos,
                titbit_layer,
                command,
                ..
            } => (
                *target_pos,
                recorded_ground_target_titbit_layer(*command, *titbit_layer),
            ),
            QaReplayCommand::ShieldRaise {
                danger_point,
                danger_point_layer,
                ..
            } => (*danger_point, *danger_point_layer),
            QaReplayCommand::Move { destination, .. }
            | QaReplayCommand::TacticalMove { destination, .. } => (
                self.world.fast_grid.convert_2d_to_3d(
                    *destination,
                    crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
                    self.sight_obstacles(assets),
                ),
                actor_layer,
            ),
            QaReplayCommand::DropAle { goal_override, .. } => (
                self.world.fast_grid.convert_2d_to_3d(
                    position,
                    crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
                    self.sight_obstacles(assets),
                ),
                goal_override
                    .map(|(_, layer)| layer)
                    .unwrap_or_else(|| panic!("recorded DropAle has no authoritative goal layer")),
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
        let supplier_handle = supplier.map(|id| ElementHandle(id.index()));
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
        if let Some(tb) = titbit_id {
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
    pub(super) fn apply_queue_quick_action(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        seat: usize,
        action: crate::profiles::Action,
        command: &PlayerCommand,
    ) {
        use PlayerCommand::*;

        if let MoveTacticalUnits {
            formation,
            soldiers,
            destination,
            running,
        } = command
        {
            let valid: Vec<_> = soldiers
                .iter()
                .copied()
                .filter(|soldier| self.is_tactically_controllable(*soldier))
                .collect();
            let leaders = self.players.seats[seat].selection.clone();
            let slots = self.tactical_formation_slots(
                assets,
                &valid,
                &leaders,
                *destination,
                *formation,
                false,
            );
            for (actor, slot) in slots {
                let outcome = self
                    .plan_recorded_group_move(assets, &[actor], slot, None, None, None)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| {
                        panic!("tactical queue planner returned no result for {actor:?}")
                    });
                let plan = match outcome {
                    PlannedRecordedGroupMoveOutcome::Resolved(plan) => plan,
                    PlannedRecordedGroupMoveOutcome::Unauthorized { actor } => {
                        tracing::warn!(
                            ?actor,
                            ?slot,
                            "planned tactical move has no authorized destination"
                        );
                        continue;
                    }
                };
                let route = plan.route;
                let before_len = self.players.auto_queues.len(actor);
                self.record_resolved_group_move_step_in_store(
                    actor,
                    plan.destination,
                    *running,
                    plan.route,
                    Some(action),
                    assets,
                    QuickActionRecordingStore::Automatic,
                );
                let step = self
                    .players
                    .auto_queues
                    .last_step_mut(actor)
                    .unwrap_or_else(|| panic!("tactical queue step for {actor:?} disappeared"));
                step.replay = crate::macro_store::QaReplayCommand::TacticalMove {
                    destination: plan.destination,
                    running: *running,
                    route,
                    formation: *formation,
                };
                self.finish_automatic_quick_action_capture(
                    sim, display, assets, actor, before_len, command,
                );
            }
            return;
        }

        if let GroupMove {
            actors,
            destination,
            running,
            goal_override,
            goal_sector_index_override,
            door_route_override,
            ..
        } = command
        {
            let plans = self.plan_recorded_group_move(
                assets,
                actors,
                *destination,
                *goal_override,
                *goal_sector_index_override,
                *door_route_override,
            );
            for plan in plans {
                let plan = match plan {
                    PlannedRecordedGroupMoveOutcome::Resolved(plan) => plan,
                    PlannedRecordedGroupMoveOutcome::Unauthorized { actor } => {
                        tracing::warn!(
                            ?actor,
                            ?destination,
                            "Shift group-move queue rejected unauthorized formation slot"
                        );
                        continue;
                    }
                };
                let before_len = self.players.auto_queues.len(plan.actor);
                self.record_resolved_group_move_step_in_store(
                    plan.actor,
                    plan.destination,
                    *running,
                    plan.route,
                    Some(action),
                    assets,
                    QuickActionRecordingStore::Automatic,
                );
                self.finish_automatic_quick_action_capture(
                    sim, display, assets, plan.actor, before_len, command,
                );
            }
            return;
        }

        let actors: Vec<EntityId> = match command {
            LaunchInteraction { actor, .. }
            | LaunchGroundTarget { actor, .. }
            | DropAleAt { actor, .. }
            | LaunchSelfAbility { actor, .. }
            | LaunchScrollRead { actor, .. }
            | EnterSwordfight { actor, .. }
            | SwordStrikeCmd { actor, .. }
            | RaiseShieldWithDanger { actor, .. } => vec![*actor],
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
                None,
                assets,
                QuickActionRecordingStore::Automatic,
            );
            self.finish_automatic_quick_action_capture(
                sim, display, assets, actor, before_len, command,
            );
        }
        if matches!(command, RaiseShieldWithDanger { .. }) {
            self.players.seats[seat].planned_shield_target = None;
        }
    }

    pub(super) fn finish_automatic_quick_action_capture(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        actor: EntityId,
        before_len: usize,
        command: &PlayerCommand,
    ) {
        if self.players.auto_queues.len(actor) != before_len + 1 {
            tracing::warn!(?actor, ?command, "Shift queue command produced no QA step");
            return;
        }

        let was_active = self.players.auto_queue_active.contains(&actor);
        if !was_active {
            self.players.auto_queue_active.push(actor);
        }
        let actor_busy = self
            .orders
            .sequence_manager
            .has_unpostponed_element_for_actor_matching(actor, |command| command != Command::Wait);
        if !was_active && !actor_busy {
            self.start_auto_queue_front(sim, display, assets, actor);
        }
    }

    pub(super) fn apply_make_queued_action_fast(
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
    pub(super) fn start_auto_queue_front(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
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
        let launched =
            self.check_quick_action_steps_validity(assets, pc, std::slice::from_ref(&entry.step))
                && self.replay_quick_action_steps(
                    sim,
                    display,
                    assets,
                    pc,
                    vec![entry.step.clone()],
                    QuickActionRecordingStore::Automatic,
                );
        if !launched {
            // Automatic queues cannot wait for a user to click a failed QA
            // item. Fizzle once, discard the invalid front item, and leave
            // the tail ready to advance on the next idle tick.
            tracing::warn!(?pc, "automatic quick action fizzled; dropping queue front");
            self.feedback
                .pending_side_effects
                .sounds
                .push(crate::engine::SoundCommand::Jingle(
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
        // The host observes the shorter independent queue on its next draw and
        // starts that actor portrait's own falling-strip easing.
        let _ = display;
    }

    /// Advance automatic Shift-click queues after actor work has settled for
    /// the frame. Called by the engine tick, not the renderer, so replay and
    /// rollback observe identical launch frames.
    pub(in crate::engine) fn advance_auto_quick_action_queues(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
    ) {
        let active = self.players.auto_queue_active.clone();
        for pc in active {
            if self
                .orders
                .sequence_manager
                .has_unpostponed_element_for_actor_matching(pc, |command| command != Command::Wait)
            {
                continue;
            }
            if !self.players.auto_queues.is_empty(pc) {
                self.start_auto_queue_front(sim, display, assets, pc);
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
    pub(super) fn apply_start_macro(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
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
            self.replay_macro_slot(sim, display, assets, *pc_id, slot);
            if self.has_quick_action(*pc_id, slot) {
                // The original game posts a macro-fizzle message when a quick action's
                // validity/launch gate fails. The game consumes that message
                // synchronously and blinks this PC's still-live QA slot. The
                // rollback-safe engine exposes that presentation mutation as
                // a typed host event rather than mutating host UI scratch.
                self.feedback.pending_side_effects.host_events.push(
                    crate::engine::HostEvent::MacroUi(crate::engine::MacroUiHostEvent::BlinkQa {
                        pc_id: *pc_id,
                        slot: slot as usize,
                    }),
                );
            }
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
            .push(crate::engine::SoundCommand::Jingle(jingle));

        // If this was an "all PCs" launch and every PC that had a macro
        // at this slot has now fired (i.e. no PC still has one), collapse
        // the strip.
        if pc.is_none() && all_launched {
            self.do_tetris_macro(slot);
        }
    }

    /// Replay one PC's macro slot — the per-PC half of [`apply_start_macro`].
    /// Extracted so the iteration above can re-borrow `self` between steps.
    pub(super) fn launch_recorded_group_move_qa(
        &mut self,
        pc: EntityId,
        destination: MapPoint,
        running: bool,
        route: crate::macro_store::RecordedQaMoveRoute,
        append_recovery: bool,
    ) {
        use crate::sequence::{Sequence, SequenceElement, SequenceElementData};

        let action = if running {
            crate::order::OrderType::RunningUpright
        } else {
            crate::order::OrderType::WalkingUpright
        };
        let mut seek = SequenceElement::new_movement(1, Command::Seek, Some(pc), action);
        let mut post_seek = Sequence::new();
        post_seek.append_element(SequenceElement::new(
            1,
            Command::SpeakHeroReachDestination,
            Some(pc),
        ));
        if append_recovery {
            self.append_posture_recovery(pc, &mut post_seek);
        }
        let SequenceElementData::Movement {
            destination: seek_destination,
            layer,
            sector,
            post_seek_sequence,
            ..
        } = &mut seek.data
        else {
            unreachable!("new_movement must create movement data")
        };
        let exact_goal = self
            .world
            .fast_grid
            .level
            .sectors
            .get(usize::from(route.goal_sector_index))
            .unwrap_or_else(|| {
                panic!(
                    "recorded QA group-move goal index {} is absent from the current level",
                    route.goal_sector_index
                )
            });
        assert_eq!(
            exact_goal.sector_number, route.goal_sector,
            "recorded QA group-move exact goal disagrees with its public sector"
        );
        let goal_sector =
            crate::position_interface::SectorHandle::new(u16::from(route.goal_sector))
                .unwrap_or_else(|| {
                    panic!(
                        "recorded QA group move has invalid public sector {}",
                        route.goal_sector
                    )
                })
                .with_arena_index(route.goal_sector_index);
        *seek_destination = destination;
        *layer = route.goal_layer;
        *sector = Some(goal_sector);
        *post_seek_sequence = Some(post_seek.into_post_seek());

        let mut sequence = Sequence::new();
        sequence.append_element(seek);
        self.launch_sequence(sequence);
    }

    pub(super) fn replay_macro_slot(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
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

        if !self.check_quick_action_steps_validity(assets, pc, &steps)
            || !self.replay_quick_action_steps(
                sim,
                display,
                assets,
                pc,
                steps,
                QuickActionRecordingStore::Manual,
            )
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
    pub(super) fn replay_quick_action_steps(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        pc: EntityId,
        steps: Vec<crate::macro_store::QuickActionStep>,
        replay_store: QuickActionRecordingStore,
    ) -> bool {
        let step_count = steps.len();
        let mut posture_recovery_embedded = false;
        for (step_index, step) in steps.into_iter().enumerate() {
            let cmd = match step.replay {
                crate::macro_store::QaReplayCommand::Move {
                    destination,
                    running,
                    route,
                } => {
                    // movement with recording enabled retained this exact
                    // SEEK/post-seek shape. Launch it directly rather than
                    // re-entering formation placement with a one-PC group.
                    self.launch_recorded_group_move_qa(
                        pc,
                        destination,
                        running,
                        route,
                        step_index + 1 == step_count,
                    );
                    if step_index + 1 == step_count {
                        posture_recovery_embedded = true;
                    }
                    continue;
                }
                crate::macro_store::QaReplayCommand::TacticalMove {
                    destination,
                    running,
                    route,
                    formation,
                } => {
                    if !self.prepare_queued_tactical_move(pc, destination, formation) {
                        return false;
                    }
                    self.launch_recorded_group_move_qa(
                        pc,
                        destination,
                        running,
                        route,
                        step_index + 1 == step_count,
                    );
                    if step_index + 1 == step_count {
                        posture_recovery_embedded = true;
                    }
                    continue;
                }
                crate::macro_store::QaReplayCommand::Interaction {
                    target,
                    command,
                    double_click,
                } => {
                    // Runtime second-line-of-defence for the per-step
                    // validity gate.  `check_quick_action_steps_validity`
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
                    // Sequence-backed QA interactions are recorded after
                    // Original has resolved the click into a concrete seek.
                    // A double-click therefore means that the stored seek is
                    // RunningUpright; it is not a raw QUICKITOS_INTERRACT
                    // click that needs the special single-click prime. Clone
                    // that one resolved route directly. Re-entering the live
                    // click dispatcher would either launch a walking route
                    // twice or reduce the running click to fast-movement conversion with no
                    // newly launched route.
                    let is_tail = step_index + 1 == step_count;
                    let was_aiming = self
                        .get_entity(pc)
                        .and_then(|entity| entity.actor_data())
                        .unwrap_or_else(|| panic!("quick-action owner {pc:?} has no actor state"))
                        .action_state
                        .is_bow();
                    let command = quick_action_tail_command(command, is_tail, was_aiming);
                    let append_recovery = is_tail && command == Command::TakeCorpse;
                    self.apply_recorded_interaction_with_seek(
                        sim,
                        pc,
                        target,
                        command,
                        double_click,
                        append_recovery,
                    );
                    posture_recovery_embedded |= append_recovery;
                    continue;
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
                    // Original stores the already-resolved scroll sequence.
                    // Rebuild that sequence with its recorded gait instead
                    // of taking the live double-click fast-movement shortcut.
                    self.apply_scroll_read_with_seek_inner(sim, pc, target, running, true);
                    continue;
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
                    already_authorized,
                    goal_override,
                    goal_sector_index_override,
                    recorded_gate_path,
                } => PlayerCommand::DropAleAt {
                    actor: pc,
                    target_pos,
                    running,
                    already_authorized,
                    goal_override,
                    goal_sector_index_override,
                    recorded_gate_path,
                },
                crate::macro_store::QaReplayCommand::Swordfight { target, running } => {
                    // See Interaction arm — whole-sequence abort on
                    // target-gone.
                    if self.get_entity(target).is_none() {
                        return false;
                    }
                    if replay_store == QuickActionRecordingStore::Automatic
                        && !self.get_entity(pc).is_some_and(Entity::is_pc)
                        && !self.prepare_queued_tactical_combat_command(pc)
                    {
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
                    composite,
                    gesture_quality,
                    with_seek,
                    seek_distance,
                } => {
                    // See Interaction arm — whole-sequence abort on
                    // target-gone.
                    if self.get_entity(target).is_none() {
                        return false;
                    }
                    if replay_store == QuickActionRecordingStore::Automatic
                        && !self.get_entity(pc).is_some_and(Entity::is_pc)
                        && !self.prepare_queued_tactical_combat_command(pc)
                    {
                        return false;
                    }
                    PlayerCommand::SwordStrikeCmd {
                        actor: pc,
                        target,
                        command,
                        composite,
                        gesture_quality,
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
                        return false;
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
                            .map(|e| e.element_data().posture())
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
                            | crate::element::Posture::Cloaked
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
            // The original game clones the recorded elements for a quick action into a
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
                self.apply_replayed_quick_action_command(sim, display, assets, &cmd, replay_store);
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

        // Post-seek continuation is implemented via
        // `ActorData::post_seek_sequence`: seek-building helpers attach
        // their continuation directly to the launched movement element,
        // so replay does not need an extra per-PC handoff here.

        true
    }

    /// Dispatch one replayed QA command through its normal live command path.
    /// Automatic queue execution temporarily hides the independently armed
    /// manual recorder: otherwise the nested command would be captured as a
    /// new manual step (and specialized recording arms would stop recording)
    /// before the automatic entry is retired.
    pub(super) fn apply_replayed_quick_action_command(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
        assets: &LevelAssets,
        command: &PlayerCommand,
        replay_store: QuickActionRecordingStore,
    ) {
        if replay_store == QuickActionRecordingStore::Manual {
            self.apply_command_authoritative(sim, display, assets, 0, command);
            return;
        }

        let armed_manual_recorders = std::mem::take(&mut self.players.qa_recording_for);
        self.apply_command_authoritative(sim, display, assets, 0, command);
        assert!(
            self.players.qa_recording_for.is_empty(),
            "automatic quick-action replay unexpectedly changed manual recording targets"
        );
        self.players.qa_recording_for = armed_manual_recorders;
    }

    /// Replay the three non-sequence quick-action variants serialized by
    /// player actor. Interact deliberately re-enters the target's
    /// state-driven click ladder instead of guessing a resolved command from
    /// the saved target kind.
    pub(super) fn replay_legacy_quickito(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut CameraDisplayState,
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
                    // The original game inserts a literal per-frame sequence update
                    // between the synthetic leading single-click and the
                    // saved double-click. At this input boundary no entity
                    // phase work remains; the normal sequence phase drains
                    // precisely the newly registered click sequence.
                    self.hourglass_phase_sequences_authoritative(sim, display, assets, &[], &[]);
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
    pub(super) fn legacy_human_mouse_clicked(
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
    /// original-game save. `Some(false)` preserves the slot after a validity
    /// failure; `Some(true)` consumed it; `None` selects normal semantic-step
    /// playback.
    pub(super) fn replay_legacy_sequence_macro(
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
                    && !valid(
                        engine,
                        assets,
                        &post_seek.clone().into_sequence(),
                        swordfighting,
                        false,
                    )
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
            actor.post_seek_sequence = Some(seek.into_post_seek());
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
    ///   * any step references a target entity that no longer exists, or an
    ///     interaction target/owner no longer satisfies Original's
    ///     per-command validity gate → fail;
    ///   * any non-MOVE/SEEK/POSTURE step while the PC is currently
    ///     swordfighting → fail.  `Move` (which expands to MOVE/SEEK
    ///     on dispatch) and `PostureToggle` survive the gate (the
    ///     posture quickitos has no swordfight restriction); recorded
    ///     interactions, sword-strikes, abilities, ground-targets,
    ///     etc. all fail.
    ///
    /// Returns `true` to allow replay, `false` to fizzle.
    pub(super) fn check_quick_action_steps_validity(
        &self,
        assets: &LevelAssets,
        pc: EntityId,
        steps: &[crate::macro_store::QuickActionStep],
    ) -> bool {
        use crate::macro_store::QaReplayCommand;
        if !self.get_entity(pc).is_some_and(Entity::is_pc) && !self.is_tactically_controllable(pc) {
            return false;
        }
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
            // Semantic QA steps stand in for the cloned Original sequence
            // element. Interactions may be materialised beneath a Seek at
            // dispatch time, but quick-action startup validates those nested
            // post-seek elements before cloning, with position checks off.
            // Reconstruct just that interaction element so changed-state
            // target and owner rules (Hit/Strangle, Bow, Take, Search) run
            // before any step can mutate the world.
            if let QaReplayCommand::Interaction {
                target, command, ..
            }
            | QaReplayCommand::TargetInteraction {
                target, command, ..
            } = &step.replay
                && matches!(
                    command,
                    Command::HitCmd
                        | Command::StrangleCmd
                        | Command::ShootBow
                        | Command::ShootBowOnce
                        | Command::Take
                        | Command::SearchCmd
                        | Command::TieCmd
                        | Command::Untie
                )
            {
                let element =
                    SequenceElement::new_interaction(1, *command, Some(pc), Some(*target));
                if !self.check_sequence_element_validity(assets, pc, &element, false) {
                    return false;
                }
            }
            // Per-element swordfight gate: while the PC is mid-fight,
            // only MOVE, SEEK, or PostureToggle may run.  `Move`
            // covers MOVE+SEEK on dispatch; `PostureToggle` enters
            // through the quickitos path which has no swordfight
            // gate, so it must also pass.
            if is_swordfighting
                && !matches!(
                    &step.replay,
                    QaReplayCommand::Move { .. }
                        | QaReplayCommand::TacticalMove { .. }
                        | QaReplayCommand::PostureToggle { .. }
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
    pub(super) fn apply_start_recording_macro(
        &mut self,
        seat: usize,
        pc: Option<EntityId>,
        slot: u8,
    ) {
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
    pub(super) fn apply_change_qa_memory(&mut self, seat: usize, slot: u8) {
        if (slot as usize) >= crate::macro_store::NUMBER_OF_QA_MEMORY {
            return;
        }
        // End recording on every PC that was armed (the currently-
        // armed set, not the current selection — those can differ).
        self.stop_recording_macro();
        // Re-arm on whoever is currently selected.
        let targets: Vec<EntityId> = self.players.seats[seat].selection.to_vec();
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
    pub(super) fn apply_delete_macro(
        &mut self,
        _display: &mut CameraDisplayState,
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
                self.do_tetris_macro(slot);
            }
        }
    }
}
