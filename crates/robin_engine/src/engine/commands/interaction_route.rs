//! Interaction seek construction, including recorded quick-action recovery.
//! Route construction retains its original sequence and actor-boundary order.

use super::object_use::take_seek_tolerance;
use crate::coordinates::MapPoint;
use crate::element::{Command, Entity, EntityId};
use crate::engine::movement::GoalShape;
use crate::engine::{EngineInner, LevelAssets};
use crate::sequence::{
    Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
};

/// Preserve movement construction's sector test after adapting an actor
/// already committed to a door onto that door's far side.
///
/// The original game performs the adaptation before comparing the goal and
/// source sectors. If the far side is already the target
/// sector, the route is a direct Move and must not gain a leading
/// AssertPosition merely because the actor's raw sector differed earlier.
pub(super) fn target_interaction_assert_source_sector(
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
    pub(super) fn actor_action_distance(
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

    pub(super) fn interaction_action_distance(
        &self,
        actor: EntityId,
        command: Command,
    ) -> Option<f32> {
        let distance = match command_action_distance_animation(command) {
            Some(animation) => self.actor_action_distance(actor, animation),
            None => Some(interaction_distance(command)),
        }?;
        // These original-game input paths explicitly narrow the action distance to
        // 16 bits before constructing the movement-assisted interaction. Other
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
                | Command::Untie
        ) {
            Some((distance as u16) as f32)
        } else {
            Some(distance)
        }
    }

    /// Launch an interaction, prepending a Seek walk if the actor is
    /// too far away or in a different sector.
    pub(super) fn apply_interaction_with_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        actor: EntityId,
        target: EntityId,
        command: Command,
        running: bool,
    ) {
        self.apply_interaction_with_seek_and_recovery(
            sim, actor, target, command, running, false, false,
        );
    }

    /// Rebuild a sequence-backed QA interaction from its recorded semantic
    /// route. The original game's quick-action start clones the stored movement
    /// element, including its running animation, rather than treating it as
    /// a fresh live double-click.
    pub(super) fn apply_recorded_interaction_with_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        actor: EntityId,
        target: EntityId,
        command: Command,
        running: bool,
        append_posture_recovery: bool,
    ) {
        self.apply_interaction_with_seek_and_recovery(
            sim,
            actor,
            target,
            command,
            running,
            append_posture_recovery,
            true,
        );
    }

    pub(super) fn stamp_beggar_dont_talk_counter(&mut self, target: EntityId) {
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
    pub(super) fn apply_interaction_with_seek_and_recovery(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        actor: EntityId,
        target: EntityId,
        command: Command,
        running: bool,
        append_posture_recovery: bool,
        recorded_quick_action: bool,
    ) {
        // Ranged actions bypass the seek entirely: the actor fires or
        // throws from wherever it stands.  This mirrors the original
        // bow click path, which launches the shoot-bow command directly.
        if matches!(
            command,
            Command::ShootBow | Command::ShootBowOnce | Command::ThrowApple | Command::ThrowStone
        ) {
            let elem = SequenceElement::new_interaction(1, command, Some(actor), Some(target));
            // The original game's input handlers launch the sequence element here.
            // That only registers the element for the sequence-manager tick,
            // after the entity loop, so an order already owned by the PC gets
            // one final Execute tick before this interaction is instructed.
            let mut seq = Sequence::new();
            seq.append_element(elem);
            self.launch_sequence(seq);
            return;
        }

        // ClimbUpOnShoulders has a multi-element post-seek that the
        // generic single-interaction path can't express:
        // `Seek(USE_POINT, tolerance=8) → [turn(L1) →
        // ClimbUpOnShoulders(L2)]`.  Route through a dedicated helper.
        if command == Command::ClimbUpOnShoulders {
            self.apply_climb_on_shoulders_with_seek(actor, target, running);
            return;
        }

        // When the click was a double-click and the PC is *not*
        // recording a macro, just convert to fast movement and drop the
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
                | Command::Untie
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

        if running
            && !is_recording_macro
            && !recorded_quick_action
            && is_addinteraction_with_seek_command
        {
            self.actor_make_fast(sim, actor);
            // Civilian click handling performs this post-call stamp even when
            // seek-interaction construction reduced the double-click to fast movement.
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
                e.element_data().posture(),
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
                    e.current_gameplay_point_map().unwrap_or(pos_map)
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
            // Player-character clicking owns a distinct in-coma-PC
            // pickup path. Unlike human click handling, it neither casts the
            // lift action distance to 16 bits nor uses it unchanged: it keeps
            // the fractional value and adds 10
            // for the pickup command.
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
        // to halt on arrival; `SEEK_IN_BUILDINGS` lets seek refresh
        // short-circuit when both actor and target are already inside
        // the same building.
        //
        // TakeCorpse carries the flag from
        // the human-actor click carry arm
        // for human actors. TODO: the PC-specific in-coma pickup builds its
        // Seek + post-seek pair by hand and deliberately does NOT pass
        // seeking within buildings; Rust routes every carry click
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
            | Command::Untie
            | Command::Take => {
                per_command_seek_flags |= MoveFlags::SEEK_IN_BUILDINGS;
            }
            Command::TakeCorpse if !pc_in_coma_carry => {
                per_command_seek_flags |= MoveFlags::SEEK_IN_BUILDINGS;
            }
            _ => {}
        }

        // Object clicks are a distinct Original path:
        // Object clicking always constructs a SEEK element and
        // hangs TAKE off its post-seek sequence, even when the actor is
        // already within the radius plus 15. The immediately-satisfied seek
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
            //     the fast-movement/animation pipeline stands up first).
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
                // Civilian clicking asks
                // seek-interaction construction to face the beggar. The original game
                // translates that boolean into use-point movement, so
                // seek refresh authorizes the PC's move box at the beggar's
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
            // initial dispatch / seek-refresh time, where the chase
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
                *post_seek_sequence = Some(post_seek.into_post_seek());
            }

            let mut seq = Sequence::new();
            seq.append_element(seek);
            self.launch_sequence(seq);
        } else {
            // Seek-based interaction builds and launches a sequence even
            // when no seek is necessary. Launching the owned element through
            // the eager single-element wrapper would arbitrate immediately,
            // before this frame's actor slot; the Original does not instruct
            // it until the sequence-manager tick at the end of the frame.
            let mut seq = Sequence::new();
            seq.append_element(interaction);
            if append_posture_recovery {
                self.append_posture_recovery(actor, &mut seq);
            }
            self.launch_sequence(seq);
        }
    }

    /// Reproduce a target's ordinary non-quick-action click route:
    /// synchronously construct movement to the target with zero tolerance,
    /// then turn to the target hotspot and perform the
    /// resolved interaction.
    pub(super) fn apply_target_interaction_route(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        actor: EntityId,
        target: EntityId,
        command: Command,
        running: bool,
    ) -> bool {
        let (actor_pos, actor_sector, actor_posture, actor_auth, door_source) = {
            let entity = self
                .get_entity(actor)
                .unwrap_or_else(|| panic!("target interaction requires missing actor {actor:?}"));
            let door_source = crate::engine::movement::current_door_for_route_source(entity);
            (
                entity.element_data().position_map(),
                crate::engine::ai::ai_view_position_sector(self, entity.element_data())
                    .unwrap_or_else(|| panic!("target interaction actor {actor:?} has no sector")),
                entity.element_data().posture(),
                entity.actor_auth_info(),
                door_source,
            )
        };
        let (target_pos, target_sector, target_layer, target_point) = {
            let entity = self
                .get_entity(target)
                .unwrap_or_else(|| panic!("target interaction requires missing target {target:?}"));
            (
                entity.element_data().position_map(),
                crate::engine::ai::ai_view_position_sector(self, entity.element_data())
                    .unwrap_or_else(|| {
                        panic!("target interaction target {target:?} has no sector")
                    }),
                entity.element_data().layer(),
                entity.current_gameplay_point_map().unwrap_or_else(|| {
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
            let (source_pos, source_sector) = door_source
                .and_then(|(door, door_direction)| {
                    crate::engine::movement::adapt_source_to_current_door_with_identity(
                        &self.script_domains.interactables.doors,
                        door,
                        door_direction,
                    )
                })
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
                    "target-element click could not construct its gate route"
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
    /// target clicking: a coordinate SEEK at the recorded
    /// target position, with no movement flags and zero tolerance, followed
    /// by the recorded Turn and interaction elements.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn replay_recorded_target_interaction(
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
            *post_seek_sequence = Some(post_seek.into_post_seek());
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

        // The original game sends the stop event directly while refreshing seek.
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
    ///   level 1: turn PC → NPC
    ///   level 1: turn NPC → PC
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
    pub(super) fn apply_scroll_read_with_seek(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        actor: EntityId,
        target: EntityId,
        running: bool,
    ) {
        self.apply_scroll_read_with_seek_inner(sim, actor, target, running, false);
    }

    pub(super) fn apply_scroll_read_with_seek_inner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        actor: EntityId,
        target: EntityId,
        running: bool,
        recorded_quick_action: bool,
    ) {
        use crate::sequence::{Field, FieldValue};

        // `running && !is_recording` is a short-circuit — the PC just
        // gets fast-movement conversion and we never build the composite.
        let is_recording = self.is_recording_macro();
        if running && !is_recording && !recorded_quick_action {
            self.actor_make_fast(sim, actor);
            return;
        }

        let (pc_pos, pc_posture) = match self.get_entity(actor) {
            Some(e) => (e.element_data().position_map(), e.element_data().posture()),
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
            // Original-game behavior:
            //   NPC click handling -> player seek-sequence addition ->
            //   quick-action assignment, then temporary action disabling only
            //   when the actor is climbing or in a building,
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

        // NPC click handling passes the literal distance 30 to
        // seek-sequence construction; it is intentionally not derived from an
        // animation profile (the Original even marks that constant FIXME).
        let action_distance = 30.0;

        // Build the composite command sequence.  Level numbers are
        // relative to this sequence; elements at the same level run
        // concurrently and advance together.  LockAi and both
        // Turn elements share level 1; UnlockAi / OpenScroll share
        // level 2. The first turn element turns the PC toward the
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
            *post_seek_sequence = Some(command_seq.into_post_seek());
        }

        let mut seq = Sequence::new();
        seq.append_element(seek);
        self.launch_sequence(seq);
    }

    /// Build `[Seek(USE_POINT, tolerance=8) → (turn(L1) →
    /// ClimbUpOnShoulders(L2))]` for a click on a HelpingToClimb PC.
    /// Skips the seek when the climber is already inside the
    /// tolerance.
    pub(super) fn apply_climb_on_shoulders_with_seek(
        &mut self,
        actor: EntityId,
        target: EntityId,
        running: bool,
    ) {
        let (pc_pos, pc_posture) = match self.get_entity(actor) {
            Some(e) => (e.element_data().position_map(), e.element_data().posture()),
            None => return,
        };
        let tgt_pos = match self.get_entity(target) {
            Some(e) => e.element_data().position_map(),
            None => return,
        };

        // Player-character clicking authors this point seek with the
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
            *post_seek_sequence = Some(command_seq.into_post_seek());
        }

        let mut seq = Sequence::new();
        seq.append_element(seek);
        self.launch_sequence(seq);
    }
}

/// Animation whose sprite-script action distance drives a
/// seek-before-interact command.
pub(crate) fn command_action_distance_animation(cmd: Command) -> Option<crate::order::OrderType> {
    use crate::order::OrderType;

    match cmd {
        Command::StrangleCmd => Some(OrderType::Strangling),
        Command::HealCmd => Some(OrderType::Healing),
        Command::TieCmd | Command::Untie => Some(OrderType::Tying),
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
pub(super) fn interaction_distance(cmd: Command) -> f32 {
    match cmd {
        Command::StrangleCmd => 30.0,
        Command::HealCmd => 35.0,
        Command::TieCmd | Command::Untie => 25.0,
        Command::TakeCorpse => 25.0,
        Command::ClimbUpOnShoulders => 8.0,
        Command::SearchCmd => 25.0,
        Command::ShootBow => 0.0, // bow has no walk-up
        Command::HitCmd => 30.0,
        Command::ThrowApple | Command::ThrowStone => 0.0, // ranged
        Command::RaiseShield => 35.0,
        // The original game's downward sword strike passes the literal `40` to
        // the seek-interaction helper.
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
