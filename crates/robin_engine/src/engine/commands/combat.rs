//! Swordfight entry and strike execution after central command validation/recording.
//! Keep RNG consumption and sequence-manager launch order local and unchanged.

use super::object_use::{coin_pickup_target, determine_use_command};
use crate::coordinates::MapPoint;
use crate::element::{Command, EntityId, Human as _};
use crate::engine::movement::GoalShape;
use crate::engine::{EngineInner, LevelAssets};
use crate::player_command::{CompositeSwordTechnique, GestureQuality};
use crate::sequence::{
    Field, FieldValue, MoveFlags, Sequence, SequenceElement, SequenceElementData,
};

/// Rebuild the seek tolerance omitted by early random-input parity schemas.
///
/// Random-input generation only sends clicks, never mouse-way drags. Therefore
/// its thrust-A command is the original game's no-pattern arm, which uses
/// the weapon's generic maximum. B-E can only be emitted by the gesture arm
/// and use their per-thrust maxima.
///
/// TODO(parity-schema): an arbitrary legacy interactive thrust-A record is
/// intrinsically ambiguous because the old command omitted its mouse pattern.
/// Keep current trace schemas' resolved `seek_distance` mandatory.
pub(super) fn legacy_random_input_sword_seek_distance(
    weapon: &crate::profiles::HtHWeaponProfile,
    strike_cmd: Command,
) -> f32 {
    let maximum = if strike_cmd == Command::SwordstrikeThrustA {
        weapon.distance[crate::weapons::WeaponDistance::Maximal as usize]
    } else {
        let strike = crate::weapons::SwordStrike::from_command(strike_cmd).unwrap_or_else(|| {
            panic!("legacy sword seek command {strike_cmd:?} is not a normal sword strike")
        });
        weapon.thrusts[strike as usize].maximal_distance
    };
    0.9_f32 * f32::from(maximum)
}

/// Author one ordinary strike or one of the optional two-strike techniques.
/// Sequential command levels give each constituent its normal animation,
/// interruption, target validation and energy cost.
fn sword_gesture_sequence(
    actor: EntityId,
    target: EntityId,
    first_command: Command,
    composite: Option<CompositeSwordTechnique>,
    gesture_quality: GestureQuality,
) -> Sequence {
    assert!(
        gesture_quality.is_strike_quality(),
        "combat gesture sequence received invalid or zero quality"
    );
    let commands: Vec<Command> = match composite {
        Some(technique) => technique.commands().into_iter().collect(),
        None => vec![first_command],
    };
    let mut sequence = Sequence::new();
    for (index, command) in commands.into_iter().enumerate() {
        assert!(
            command.is_swordstrike(),
            "combat gesture authored non-sword command {command:?}"
        );
        let mut element = SequenceElement::new_interaction(
            u16::try_from(index + 1).expect("two-strike command level fits u16"),
            command,
            Some(actor),
            Some(target),
        );
        element.gesture_quality = gesture_quality;
        sequence.append_element(element);
    }
    sequence
}

impl EngineInner {
    pub(super) fn dispatch_player_sword_strike(
        &mut self,
        assets: &LevelAssets,
        actor: &EntityId,
        target: &EntityId,
        command: &Command,
        composite: &Option<CompositeSwordTechnique>,
        gesture_quality: &GestureQuality,
        with_seek: &bool,
        seek_distance: &Option<f32>,
    ) {
        tracing::trace!(
            ?actor,
            ?target,
            ?command,
            ?composite,
            quality_permille = gesture_quality.permille(),
            with_seek,
            "PlayerCommand::SwordStrikeCmd"
        );
        self.prepare_tactical_player_combat_command(*actor);
        if *with_seek {
            self.apply_sword_strike_with_seek(
                assets,
                *actor,
                *target,
                *command,
                *composite,
                *gesture_quality,
                *seek_distance,
            );
        } else {
            let sequence =
                sword_gesture_sequence(*actor, *target, *command, *composite, *gesture_quality);
            // Original mouse-command handling calls
            // sequence-manager element launch here. A
            // preference strike is therefore registered for the
            // post-entity manager drain; it does not arbitrate
            // against and interrupt the actor's current order on the
            // input callback stack.
            self.launch_sequence(sequence);
        }
    }

    pub(super) fn apply_enter_swordfight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        pc_id: EntityId,
        target_id: EntityId,
        running: bool,
    ) {
        use crate::element::Entity;
        use crate::order::OrderType;

        self.prepare_tactical_player_combat_command(pc_id);

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
            let selector_camp = self
                .get_entity(pc_id)
                .unwrap_or_else(|| {
                    panic!("selected PC {pc_id:?} disappeared during sword-target dispatch")
                })
                .camp();
            let is_blipped = target.element_data().blipped;
            let is_dead = target.is_dead();
            let is_unconscious = target.human_data().is_some_and(|h| h.unconscious);
            let (is_hostile, scroll_attached) = match target {
                Entity::Soldier(s) => (
                    self.camps_are_hostile(s.camp(), selector_camp),
                    s.npc.attached_scroll.is_some(),
                ),
                _ => (false, false),
            };
            !is_blipped && !is_dead && !is_unconscious && is_hostile && !scroll_attached
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
        //   dbl-click + !record → fast movement (handled before we get
        //                                        here, via the PC command)
        // The PC must seek in a non-combat animation; a sword animation
        // here would force action_state = MovingSword at seek dispatch
        // (tick.rs), visually starting the fight while still out of
        // range.  EnterSwordfight flips the action state to sword mode
        // once the seek completes.
        let action_style = if running {
            OrderType::RunningUpright
        } else {
            match self.get_entity(pc_id).map(|e| e.element_data().posture()) {
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
        // crosses them.  Route through `launch_gate_movement_sequence`
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
            let door_source = self
                .get_entity(pc_id)
                .and_then(crate::engine::movement::current_door_for_route_source);
            let (adj_src_pos, adj_src_sector) = {
                let adapted = door_source.and_then(|(door_handle, door_direction)| {
                    crate::engine::movement::adapt_source_to_current_door(
                        &self.script_domains.interactables.doors,
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
                // entity SEEK below, whose translation-to-seek-refresh lowering
                // stamps TIME_SEEK_REFRESH, passes the victim to every gate
                // approach, and retains ENTER_SWORDFIGHT as actor-owned
                // post-seek work exactly like the Original. Arrival speech
                // and generic posture recovery belong to PC group moves, not
                // soldier interaction.
                self.launch_gate_movement_order(sim, crate::engine::movement::GateRouteRequest { entity_id: pc_id, source_sector: Some(
                        crate::position_interface::SectorHandle::new(adj_src_sector)
                            .unwrap_or_else(|| {
                                panic!(
                                    "swordfight route for {pc_id:?} adapted to invalid source sector {adj_src_sector}"
                                )
                            }),
                    ), gate_path: path, goal: goal_shape, goal_layer: arrival_layer, base_action: action_style, move_after_last_door: true, speed_factor: 1.0, initial_flags: MoveFlags::empty(), prefix_elements: Vec::new(), tail_elements: vec![enter_elem], append_arrival_speech: false, append_recovery: false });
                return;
            }

            // No usable gate path.  When `find_path_gates` fails for
            // a PC, play command-failure speech
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
            *post_seek_sequence = Some(post_seek.into_post_seek());
        }

        let mut sequence = Sequence::new();
        sequence.append_element(seek_elem);
        self.launch_sequence(sequence);
    }

    pub(super) fn apply_table_swordfight(
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

    pub(super) fn apply_sword_strike_with_seek(
        &mut self,
        assets: &LevelAssets,
        pc_id: EntityId,
        target_id: EntityId,
        strike_cmd: Command,
        composite: Option<crate::player_command::CompositeSwordTechnique>,
        gesture_quality: crate::player_command::GestureQuality,
        resolved_seek_distance: Option<f32>,
    ) {
        use crate::order::OrderType;

        let target_distance = resolved_seek_distance.unwrap_or_else(|| {
            let weapon_id = self
                .get_entity(pc_id)
                .and_then(|entity| {
                    crate::engine::melee::get_hth_weapon_id_full(
                        entity,
                        &assets.profile_manager,
                    )
                })
                .unwrap_or_else(|| {
                    panic!(
                        "legacy SwordStrikeCmd for {pc_id:?} against {target_id:?} has no equipped HtH weapon profile"
                    )
                });
            let weapon = assets
                .profile_manager
                .get_hth_weapon(weapon_id)
                .unwrap_or_else(|| {
                    panic!(
                        "legacy SwordStrikeCmd for {pc_id:?} against {target_id:?} resolves missing HtH weapon profile {weapon_id}"
                    )
                });
            legacy_random_input_sword_seek_distance(weapon, strike_cmd)
        });
        assert!(
            target_distance.is_finite() && target_distance >= 0.0,
            "resolved sword seek distance must be finite and non-negative"
        );

        let same_sector = match (self.get_entity(pc_id), self.get_entity(target_id)) {
            (Some(pc), Some(target)) => {
                pc.element_data().sector() == target.element_data().sector()
            }
            _ => false,
        };

        let mut strike_sequence =
            sword_gesture_sequence(pc_id, target_id, strike_cmd, composite, gesture_quality);
        if same_sector {
            for element in &mut strike_sequence.elements {
                element.command_level += 1;
            }
        }

        if !same_sector {
            self.launch_sequence(strike_sequence);
            return;
        }

        if !matches!(
            strike_cmd,
            Command::SwordstrikeThrustA
                | Command::SwordstrikeThrustB
                | Command::SwordstrikeThrustC
                | Command::SwordstrikeThrustD
                | Command::SwordstrikeThrustE
        ) {
            tracing::warn!(
                ?pc_id,
                ?target_id,
                ?strike_cmd,
                "apply_sword_strike_with_seek: unsupported seek strike requested; launching direct strike"
            );
            self.launch_sequence(strike_sequence);
            return;
        }

        // Swordfight gesture handling authors this seek as
        // running-with-sword handling. The forced-sword-movement flag is a
        // separate policy bit and must remain clear: if the opponent goes
        // away before Execute, Human's ordinary orphan-sword guard still
        // aborts the movement and quits swordfight.
        let mut seek_elem = SequenceElement::new_movement(
            1,
            Command::Seek,
            Some(pc_id),
            OrderType::RunningWithSword,
        );
        let post_seek = strike_sequence;
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
            *post_seek_sequence = Some(post_seek.into_post_seek());
        }

        let mut sequence = Sequence::new();
        sequence.append_element(seek_elem);
        // Sequence-element launch registers this seek at the sequence manager's
        // tail. It does not arbitrate it synchronously against an older
        // postponed chain: if that chain is released before the update reaches
        // this new seek, the older successor is instructed first.
        self.launch_sequence(sequence);
    }
}
