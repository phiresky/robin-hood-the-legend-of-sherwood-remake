//! Officer/soldier alert coordination: `alert_soldiers`,
//! `alert_officer`, `tower_guard_call_alert`, `run_and_alert_soldiers`,
//! `command_soldiers_to_attack`, `officer_look_for_soldier`, the
//! soldier-formation layout helper `can_put_soldiers_in_this_direction`,
//! the friend-list builder `create_list_of_soldiers_you_can_alert`,
//! and the report-merging helper `get_report_from_soldier`.

use crate::ai::*;
use crate::coordinates::MapPoint;
use crate::parameters_ai;
use crate::position_interface::INVERSE_ASPECT_RATIO;

use super::util::{soldier_is_able_to_help_state, vec_to_sector};
use super::{CampSoldierInfo, EnemyAi, ProfileRank, SeekFlags, combat, task_priority};

impl EnemyAi {
    /// CanPutSoldiersInThisDirection. Lays out `num_soldiers`
    /// gather slots in a line formation radiating from `pt_officer` in
    /// `direction` (16-sector compass): the front row starts 50 units
    /// ahead, each further row is offset 30 units deeper, and within a
    /// row soldiers alternate 50 units left/right.  The row length
    /// starts at `STANDARD_LINE_LENGTH` and bumps up if that would
    /// leave a single soldier in the last row.  Every slot is
    /// straight-line reachable from the officer via
    /// [`crate::fast_find_grid::FastFindGrid::is_straight_movement_authorized`].
    /// Returns `None` as soon as any slot fails the reachability test;
    /// returns `Some(slots)` on success (slot 0 is the centre of the
    /// front row, then alternating sideways within the row, then
    /// wrapping into the next row backward).
    #[allow(clippy::too_many_arguments)]
    fn can_put_soldiers_in_this_direction(
        &self,
        ctx: &AiContext,
        global: &AiGlobalState,
        tick: &AiPerTickData,
        pt_officer: MapPoint,
        direction: u16,
        num_soldiers: u16,
        grid: &crate::fast_find_grid::FastFindGrid,
    ) -> Option<Vec<Position>> {
        // Bump the line length so the last row never has a single
        // lonely soldier.
        let mut modulo = combat::STANDARD_LINE_LENGTH.max(1) as u16;
        if num_soldiers > 1 {
            while num_soldiers % modulo == 1 {
                modulo += 1;
            }
        }

        // Forward / backward / sideways iso-space
        // direction vectors.  Sideways is `(direction + 4) % 16`.
        let d = (direction & 15) as i16;
        let sideways_sector = (d + 4).rem_euclid(16);
        let v_fwd = crate::position_interface::sector_to_vector_iso(d);
        let forward_50 = (v_fwd[0] * 50.0, v_fwd[1] * 50.0);
        let backward_30 = (v_fwd[0] * 30.0, v_fwd[1] * 30.0);
        let v_side = crate::position_interface::sector_to_vector_iso(sideways_sector);
        let side_50 = (v_side[0] * 50.0, v_side[1] * 50.0);

        // When the officer is in a building, the slots live on the
        // outside layer/sector reached through their exit door.  We
        // surface those through `tick.my_exit_door` (populated by
        // `build_npc_tick_data`).  When the officer is outdoors, use
        // the officer's own layer/sector.
        let _ = global;
        let (layer, sector_handle): (u16, Option<crate::position_interface::SectorHandle>) =
            if ctx.in_building {
                let door = tick.my_exit_door?;
                (door.layer_out, door.sector_out)
            } else {
                (ctx.position.level, ctx.position.sector)
            };

        let centre = (pt_officer.x + forward_50.0, pt_officer.y + forward_50.1);
        let officer_pt = pt_officer;

        let mut positions = Vec::with_capacity(num_soldiers as usize);
        for i in 0..num_soldiers {
            let backward_index = (i / modulo) as f32;
            let rest = i % modulo;
            // Odd → (rest+1)/2; even → -(rest/2).
            let sideways_index = if rest & 1 == 1 {
                rest.div_ceil(2) as f32
            } else {
                -((rest / 2) as f32)
            };

            let px = centre.0 + sideways_index * side_50.0 + backward_index * backward_30.0;
            let py = centre.1 + sideways_index * side_50.1 + backward_index * backward_30.1;
            let slot_pt = MapPoint::new(px, py);

            if !grid.is_straight_movement_authorized(officer_pt, slot_pt, layer, &ctx.move_box) {
                return None;
            }

            positions.push(Position {
                x: px,
                y: py,
                sector: sector_handle,
                level: layer,
            });
        }

        Some(positions)
    }

    // -----------------------------------------------------------------------
    // CommandSoldiersToAttack — officer orders nearby soldiers to attack
    // Port of the legacy officer attack broadcast.
    // -----------------------------------------------------------------------

    pub fn command_soldiers_to_attack(
        &mut self,
        center: Position,
        global: &AiGlobalState,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        debug_assert_eq!(self.get_rank(), ProfileRank::Officer);

        let my_pos = ctx.position;
        self.base.seek_position = center;
        self.current_task_priority = task_priority::ALERT;

        let alert_radius = combat::ALERT_RADIUS as f32;
        let alert_radius_sq = alert_radius * alert_radius;

        self.alerted_us.clear();
        // Per-soldier positions cached for nearest-slot distribution
        // after `can_put_soldiers_in_this_direction` succeeds.
        let mut alerted_positions: Vec<Position> = Vec::new();
        let mut alerted_count: u16 = 0;
        // Accumulate a sum of normalized (friend - me) vectors so
        // that after the loop the officer faces the average direction
        // of the alerted soldiers before gathering them.
        // Updated while scanning helpers so the officer can face the
        // average helper direction.
        let mut avg_dir_vec_x: f32 = 0.0;
        let mut avg_dir_vec_y: f32 = 0.0;

        for cs in &tick.camp_soldiers {
            // Must be rank SOLDIER, able to help.
            if cs.rank != ProfileRank::Soldier || !cs.is_able_to_help {
                continue;
            }
            // MaxNorm distance check
            let dx = (cs.position.x - my_pos.x).abs();
            let dy = (cs.position.y - my_pos.y).abs();
            if dx.max(dy) >= alert_radius {
                continue;
            }
            // SquareNorm distance check
            if dx * dx + dy * dy >= alert_radius_sq {
                continue;
            }

            // Send CALL_COMBAT_ALERT with the target position
            self.base
                .pending_cross_npc_actions
                .push(CrossNpcAction::SendStimulus {
                    fallback_to_sender: None,
                    to_whole_patrol: false,
                    target: cs.handle,
                    stimulus_type: StimulusType::CallCombatAlert,
                    info: StimulusInfo::Position(center),
                });
            self.alerted_us.push(cs.handle);
            alerted_positions.push(cs.position);
            alerted_count += 1;

            // Accumulate normalized (friend - me) for the average
            // turn-towards-soldiers direction.
            let fdx = cs.position.x - my_pos.x;
            let fdy = cs.position.y - my_pos.y;
            let len = (fdx * fdx + fdy * fdy).sqrt();
            if len > 0.0 {
                avg_dir_vec_x += fdx / len;
                avg_dir_vec_y += fdy / len;
            }
        }

        if alerted_count > 0 {
            // Try a line formation on the average
            // soldier-direction side, then distribute slots to each
            // alerted soldier (nearest-slot match, outdoor only) with
            // `InstructGatherPosition`.  Indoor (door-step) variant is
            // not yet wired — without it, `command_soldiers_to_attack`
            // from inside a building still falls back to the plain
            // `CallCombatAlert` broadcast and the existing turn/gather
            // sequence below.
            if !ctx.in_building
                && let Some(grid) = grid
            {
                let avg_dir_start = vec_to_sector(avg_dir_vec_x, avg_dir_vec_y);
                let mut slots: Option<Vec<Position>> = None;
                let mut slot_direction: u16 = avg_dir_start;
                for offset in 0..16u16 {
                    let try_dir = (avg_dir_start + offset) & 15;
                    if let Some(p) = self.can_put_soldiers_in_this_direction(
                        ctx,
                        global,
                        tick,
                        MapPoint::new(my_pos.x, my_pos.y),
                        try_dir,
                        alerted_count,
                        grid,
                    ) {
                        slots = Some(p);
                        slot_direction = try_dir;
                        break;
                    }
                }

                if let Some(mut slots) = slots {
                    // Direction--; direction ^= 8.
                    // — the loop postincrements past the last-tried
                    // value, so the "correct" facing for each soldier
                    // is the opposite of the tried direction.
                    let face_threat = slot_direction ^ 8;
                    // Nearest-slot match per soldier,
                    // removing each slot as it's claimed.
                    for (i, &handle) in self.alerted_us.iter().enumerate() {
                        if slots.is_empty() {
                            break;
                        }
                        let soldier_pos = alerted_positions[i];
                        let mut best_idx = 0;
                        let mut best_sq = f32::INFINITY;
                        for (k, slot) in slots.iter().enumerate() {
                            let sx = slot.x - soldier_pos.x;
                            let sy = (slot.y - soldier_pos.y)
                                * crate::position_interface::INVERSE_ASPECT_RATIO;
                            let sq = sx * sx + sy * sy;
                            if sq < best_sq {
                                best_sq = sq;
                                best_idx = k;
                            }
                        }
                        let chosen = slots.remove(best_idx);
                        self.base.pending_cross_npc_actions.push(
                            CrossNpcAction::InstructGatherPosition {
                                target: handle,
                                position: chosen,
                                direction: face_threat,
                            },
                        );
                    }
                }
            }

            self.base.stop_all();

            // Build the turn/gather/point sequence.  If the enemy
            // (`center`) is further than 150 units (MaxNorm) from the
            // officer, first turn toward the average soldier direction
            // and gather them; then always point to the target.
            use crate::element::Command;
            use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

            let me_to_target_x = center.x - my_pos.x;
            let me_to_target_y = center.y - my_pos.y;
            let target_dir = vec_to_sector(me_to_target_x, me_to_target_y);

            let owner = self.base.owner_entity_id;
            let mut seq = Sequence::new();
            let mut level: u16 = 1;

            let enemy_max_norm = me_to_target_x.abs().max(me_to_target_y.abs());
            if enemy_max_norm > 150.0 {
                // Turn to the soldiers (face average direction).
                let avg_dir = vec_to_sector(avg_dir_vec_x, avg_dir_vec_y);
                let mut turn_elem = SequenceElement::new_generic(level, Command::Turn, owner);
                turn_elem.set_property(Field::Direction, FieldValue::Integer(avg_dir as u32));
                seq.append_element(turn_elem);
                level += 1;

                // Gather the soldiers (no properties; reference uses the
                // plain `RHSequenceElement` ctor here).
                seq.append_element(SequenceElement::new(level, Command::GatherSoldiers, owner));
                level += 1;
            }

            // Point to the target.
            let mut point_elem = SequenceElement::new_generic(level, Command::Point, owner);
            point_elem.set_property(Field::Direction, FieldValue::Integer(target_dir as u32));
            seq.append_element(point_elem);

            self.base.pending_launch_sequences.push(seq);

            self.base
                .set_transient_emoticon(EmoticonType::XMark, 20, ctx.frame);
            self.set_state(AiState::Attacking, Substate::AttackingOfficerGivingOrders);
            self.base.launch_timer(20, ctx.frame);
            self.base.friends_are_alerted = true;
            return true;
        }

        // No soldiers alerted
        false
    }

    // -----------------------------------------------------------------------
    // AlertSoldiers — officer alerts nearby soldiers and gathers them
    // Port of the legacy officer gather-alert flow.
    // -----------------------------------------------------------------------

    /// Officer alerts nearby soldiers for a seek.  Distinct from
    /// [`Self::command_soldiers_to_attack`], which is the battle-decision
    /// "everyone attack now" broadcast.  `AlertSoldiers` builds a gather
    /// group for follow-on seek coordination, sends `CALL_ALERT` (not
    /// `CALL_COMBAT_ALERT`), merges the officer's reconnaissance report
    /// into each alerted soldier, and transitions the officer into the
    /// `SeekingOfficerWaitForGroup` flow.
    pub fn alert_soldiers(
        &mut self,
        center: Position,
        flags: u16,
        global: &AiGlobalState,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Stash seek center + flags on the AI.
        let my_pos = ctx.position;
        self.base.seek_position = center;
        let incoming_flags = SeekFlags::from_bits_truncate(flags);
        self.seek_flags = incoming_flags;

        // SEEK_DELAY early-return — defer the actual
        // alert for 30 frames via SUBSTATE_SEEKING_OFFICER_CALL_GROUP.
        // (`DELAY` is the local name for SEEK_DELAY = 0x0080.)
        if incoming_flags.contains(SeekFlags::DELAY) {
            self.set_state(AiState::Seeking, Substate::SeekingOfficerCallGroup);
            self.base.set_emoticon(EmoticonType::XMark);
            self.base.launch_timer(30, ctx.frame);
            return true;
        }

        // Focus(NULL) — clear focus target.
        self.base.pending_unfocus = true;

        self.current_task_priority = task_priority::ALERT;

        // Reset the alerted / staying / us lists.
        self.alerted_us.clear();
        self.base.list_alerted_us.clear();
        self.base.list_staying_us.clear();
        self.base.list_us.clear();

        debug_assert_eq!(self.get_rank(), ProfileRank::Officer);

        let alert_radius = combat::ALERT_RADIUS as f32;
        let alert_radius_sq = alert_radius * alert_radius;

        // Officer's cached reconnaissance report — snapshot once so we
        // can broadcast a stable copy to each soldier via
        // `ConsiderReport{UPDATE_CHARLY | UPDATE_TYPE}`.
        let my_report = self.base.my_reconnaissance_report.clone();

        let my_handle = self.base.me;
        // Whether the officer is currently inside a building.
        let officer_in_building = ctx.in_building;

        // Collect alerted soldiers as (handle, sqr_distance) so we can
        // Insert in ascending-distance order.
        let mut alerted: Vec<(HumanHandle, f32)> = Vec::with_capacity(20);

        // Sum of normalized (friend - me) vectors — only used outdoors
        // To pick the group-gather direction.
        let mut avg_dir_vec_x: f32 = 0.0;
        let mut avg_dir_vec_y: f32 = 0.0;

        for cs in &tick.camp_soldiers {
            // Hard cap of 20 alerted friends.
            if alerted.len() >= 20 {
                break;
            }
            // Rank SOLDIER.
            if cs.rank != ProfileRank::Soldier {
                continue;
            }
            // is_able_to_help.
            if !cs.is_able_to_help {
                continue;
            }
            // is_allowed_to_leave_his_post || patrol_chief == me.
            // `Q_SHALL_I_STAY_ON_MY_POST`
            // returns true outdoors for tower guards, duty soldiers, and
            // company-100 soldiers; indoors (L11057) it always returns
            // false, so they're always allowed to leave.
            let cs_in_building = ctx
                .entity_view(cs.handle)
                .map(|v| v.in_building)
                .unwrap_or(false);
            let stays_on_post =
                !cs_in_building && (cs.is_tower_guard || cs.duty_flag || cs.company_number == 100);
            let allowed_to_leave = !stays_on_post;
            let patrol_chief_is_me = cs.patrol_chief != 0 && cs.patrol_chief == my_handle;
            if !(allowed_to_leave || patrol_chief_is_me) {
                continue;
            }

            // CanCallThisSoldier — reject soldiers whose
            // patrol chief is someone else (within 700 units) or who
            // are already in a conversation with another antagonist.
            if !self.can_call_this_soldier(cs, ctx, tick) {
                continue;
            }

            // MaxNorm + SquareNorm radius gates.
            let dx = (cs.position.x - my_pos.x).abs();
            let dy = (cs.position.y - my_pos.y).abs();
            if dx.max(dy) >= alert_radius {
                continue;
            }
            let sqr_dist = dx * dx + dy * dy;
            if sqr_dist >= alert_radius_sq {
                continue;
            }

            // friend.Think(stimulus) — deliver CALL_ALERT.
            // Predict the soldier's Think return value on this side so
            // the cross-NPC dispatch result matches the reference's synchronous
            // gate.  Soldiers whose state / task-priority would refuse
            // the alert are not added to `mlistAlertedUs`, do not get
            // `InstructGatherPosition`, and don't trigger
            // `ConsiderReport`.
            if !crate::ai_enemy::soldier_would_react_to_call_alert(cs) {
                continue;
            }
            self.base
                .pending_cross_npc_actions
                .push(CrossNpcAction::SendStimulus {
                    fallback_to_sender: None,
                    to_whole_patrol: false,
                    target: cs.handle,
                    stimulus_type: StimulusType::CallAlert,
                    info: StimulusInfo::None,
                });

            // Distance-sorted insertion: soldiers
            // appear in ascending SqrDistance.
            let pos = alerted
                .iter()
                .position(|&(_, d)| sqr_dist < d)
                .unwrap_or(alerted.len());
            alerted.insert(pos, (cs.handle, sqr_dist));

            // Broadcast the officer's report back so each
            // soldier picks up charly / report type.
            self.base
                .pending_cross_npc_actions
                .push(CrossNpcAction::ConsiderReport {
                    target: cs.handle,
                    report: my_report.clone(),
                    flags: crate::ai_enemy::ReportUpdateFlags::UPDATE_CHARLY.bits()
                        | crate::ai_enemy::ReportUpdateFlags::UPDATE_TYPE.bits(),
                });

            // Outdoor gather-direction accumulator.
            if !officer_in_building {
                let fdx = cs.position.x - my_pos.x;
                let fdy = cs.position.y - my_pos.y;
                let len = (fdx * fdx + fdy * fdy).sqrt();
                if len > 0.0 {
                    avg_dir_vec_x += fdx / len;
                    avg_dir_vec_y += fdy / len;
                }
            }
        }

        // Publish the alerted-soldier list.
        self.alerted_us = alerted.iter().map(|&(h, _)| h).collect();

        // Indoor officer with no stored my_door
        // bails out — no way to position soldiers outside.
        if officer_in_building && tick.my_exit_door.is_none() {
            return false;
        }

        if self.alerted_us.is_empty() {
            // Alert didn't succeed.
            return false;
        }

        let alerted_count = self.alerted_us.len() as u16;

        // Indoor door-vector. When the officer
        // is inside a building, the gather direction is biased
        // toward the door's outside vector and the door-step
        // extrapolation walks `point_out + k * door_vector` for
        // `k = 0..10`.
        let (avg_dir_start, indoor_door_geom) = if officer_in_building {
            let door = tick.my_exit_door.expect("checked above");
            // door_vector = point_out - point_mid.
            let vdx = door.point_out.x - door.point_mid.x;
            let vdy = door.point_out.y - door.point_mid.y;
            // Normalise with ASPECT_RATIO, then scale by 30.
            // We follow the same convention so the step distance matches
            // The magnitude only matters for the door-step march.
            let len = (vdx * vdx
                + vdy
                    * vdy
                    * crate::position_interface::INVERSE_ASPECT_RATIO
                    * crate::position_interface::INVERSE_ASPECT_RATIO)
                .sqrt();
            let (nx, ny) = if len > 1e-6 {
                (vdx / len, vdy / len)
            } else {
                (1.0, 0.0)
            };
            let step = (nx * 30.0, ny * 30.0);
            // Average direction = door_vector sector.
            let avg_dir = vec_to_sector(vdx, vdy);
            (avg_dir, Some((door, step)))
        } else {
            (vec_to_sector(avg_dir_vec_x, avg_dir_vec_y), None)
        };

        // Try directions / door-step positions
        // until `CanPutSoldiersInThisDirection` succeeds.
        let mut chosen_slots: Option<Vec<Position>> = None;
        let mut chosen_direction: u16 = avg_dir_start;
        let mut chosen_officer_pt: MapPoint = MapPoint::new(my_pos.x, my_pos.y);
        let mut chosen_officer_position: Position = my_pos;

        if let Some(grid) = grid {
            if let Some((door, step)) = indoor_door_geom {
                // Indoor: walk up to 10 door-step positions outside,
                // each tested against 16 directions.
                let mut try_pt = door.point_out;
                let door_pt_out = door.point_out;
                let outside_layer = door.layer_out;
                'outer: for k in 0..10u16 {
                    if k > 0
                        && !grid.is_straight_movement_authorized(
                            door_pt_out,
                            try_pt,
                            outside_layer,
                            &ctx.move_box,
                        )
                    {
                        // Blocked door-step → bail.
                        break;
                    }
                    for offset in 0..16u16 {
                        let try_dir = (avg_dir_start + offset) & 15;
                        if let Some(slots) = self.can_put_soldiers_in_this_direction(
                            ctx,
                            global,
                            tick,
                            try_pt,
                            try_dir,
                            alerted_count,
                            grid,
                        ) {
                            chosen_slots = Some(slots);
                            chosen_direction = try_dir;
                            chosen_officer_pt = try_pt;
                            // Officer's future
                            // position = doorPositionOut overlaid with
                            // try-point x/y.
                            chosen_officer_position = Position {
                                x: try_pt.x,
                                y: try_pt.y,
                                sector: door.sector_out,
                                level: door.layer_out,
                            };
                            break 'outer;
                        }
                    }
                    try_pt.x += step.0;
                    try_pt.y += step.1;
                }
            } else {
                // Outdoor: sweep 16 directions starting at the
                // average soldier-direction.
                for offset in 0..16u16 {
                    let try_dir = (avg_dir_start + offset) & 15;
                    if let Some(slots) = self.can_put_soldiers_in_this_direction(
                        ctx,
                        global,
                        tick,
                        MapPoint::new(my_pos.x, my_pos.y),
                        try_dir,
                        alerted_count,
                        grid,
                    ) {
                        chosen_slots = Some(slots);
                        chosen_direction = try_dir;
                        break;
                    }
                }
            }
        }

        let _ = chosen_officer_pt;

        let placement_ok = chosen_slots.is_some();

        // When the formation succeeded,
        // distribute slots to alerted soldiers via nearest-slot
        // match (outdoor) or slot 0 (indoor) and emit
        // `InstructGatherPosition`.  The face direction is
        // `direction ^ 8` (face the threat).
        if let Some(mut slots) = chosen_slots.clone() {
            let face_threat = chosen_direction ^ 8;
            let alerted_handles = self.alerted_us.clone();
            // Snapshot positions for the nearest-slot match
            // (outdoor branch) before we start mutating the slot
            // list.
            for &handle in &alerted_handles {
                if slots.is_empty() {
                    break;
                }
                if officer_in_building {
                    // Indoor → always slot 0.
                    let chosen = slots.remove(0);
                    self.base.pending_cross_npc_actions.push(
                        CrossNpcAction::InstructGatherPosition {
                            target: handle,
                            position: chosen,
                            direction: face_threat,
                        },
                    );
                } else {
                    // Outdoor nearest-slot match.
                    let soldier_pos = tick
                        .camp_soldiers
                        .iter()
                        .find(|cs| cs.handle == handle)
                        .map(|cs| cs.position)
                        .unwrap_or_default();
                    let mut best_idx = 0usize;
                    let mut best_sq = f32::INFINITY;
                    for (k, slot) in slots.iter().enumerate() {
                        let sx = slot.x - soldier_pos.x;
                        let sy = (slot.y - soldier_pos.y)
                            * crate::position_interface::INVERSE_ASPECT_RATIO;
                        let sq = sx * sx + sy * sy;
                        if sq < best_sq {
                            best_sq = sq;
                            best_idx = k;
                        }
                    }
                    let chosen = slots.remove(best_idx);
                    self.base.pending_cross_npc_actions.push(
                        CrossNpcAction::InstructGatherPosition {
                            target: handle,
                            position: chosen,
                            direction: face_threat,
                        },
                    );
                }
            }
        }

        if !officer_in_building {
            // Outdoor alert.
            use crate::element::Command;
            use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

            self.base.stop_all();

            // SetProperty(DIRECTION, direction ^ 8).
            // After L12296 `uwDirection ^= 8`, `uwDirection` is
            // `match_dir ^ 8`, so `uwDirection ^ 8` resolves to
            // `match_dir` — the officer turns toward the formation
            // (i.e. toward the soldiers) before the GatherSoldiers
            // animation.  In our Rust naming, `chosen_direction` is
            // already `match_dir`, so we use it verbatim (not XORed).
            // Falls back to the average soldier-direction when no
            // placement was found.
            let turn_dir = if placement_ok {
                chosen_direction
            } else {
                avg_dir_start
            };
            let owner = self.base.owner_entity_id;
            let mut seq = Sequence::new();

            // Turn toward the soldiers / threat.
            let mut turn_elem = SequenceElement::new_generic(1, Command::Turn, owner);
            turn_elem.set_property(Field::Direction, FieldValue::Integer(turn_dir as u32));
            seq.append_element(turn_elem);

            // Gather the soldiers.
            seq.append_element(SequenceElement::new(2, Command::GatherSoldiers, owner));

            self.base.pending_launch_sequences.push(seq);

            self.base.say(Remark::OfficerCallsGroup);

            self.base
                .set_transient_emoticon(EmoticonType::XMark, 20, ctx.frame);
            self.set_state(AiState::Seeking, Substate::SeekingOfficerWaitForGroup);
            self.base.launch_timer(20, ctx.frame);
        } else if placement_ok {
            // Indoor alert, found a free spot outside.
            // Stash the gather destination for the leave-house
            // substate transition.
            // gather_direction = direction ^ 8.
            // after the L12296 XOR, `uwDirection` is `match_dir ^ 8`,
            // so the stored value is `match_dir` (officer faces
            // toward the formation when leaving the house).  In our
            // Rust naming `chosen_direction` is already `match_dir`,
            // so we store it verbatim.
            self.gather_position = chosen_officer_position;
            self.gather_direction = chosen_direction;
            self.set_state(
                AiState::Seeking,
                Substate::SeekingOfficerWaitInsideHouseToInstructGroup,
            );
            self.base.launch_timer(50, ctx.frame);
        } else {
            // Indoor alert, no place outside.
            self.set_state(AiState::Seeking, Substate::SeekingOfficerWaitForGroup);
            self.base.launch_timer(20, ctx.frame);
        }

        true
    }

    // -----------------------------------------------------------------------
    // GetReportFromSoldier — officer processes a soldier's report
    // Port of the legacy soldier-report merge flow.
    // -----------------------------------------------------------------------

    /// Process a report from a soldier. Returns `true` if the report is
    /// alerting (i.e. more important than what the officer already knew),
    /// which transitions to `SeekingOfficerGetAlertingReportFromSoldier`.
    pub(super) fn get_report_from_soldier(
        &mut self,
        soldier_handle: HumanHandle,
        already_sent_out_soldiers: bool,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        let Some(soldier) = tick
            .camp_soldiers
            .iter()
            .find(|cs| cs.handle == soldier_handle)
        else {
            return false;
        };

        let my_old_report_type = self.base.my_reconnaissance_report.report_type;
        let soldier_report_type = soldier.report_type;
        let soldier_seek_position = soldier.report_seek_position;

        // Full merging: bodies, charly handle, and report type/position.
        // `consider_report_merged` also runs the side effects —
        // per-body `DeleteDetectable(DETECTABLE_BODY)` and per-charly
        // `AddDetectable(DETECTABLE_MISSED_FRIEND)` go through the
        // pending detectable queues.
        let soldier_report = crate::ai::ReconnaissanceReport {
            report_type: soldier_report_type,
            seek_position: soldier_seek_position,
            seen_bodies: soldier.report_seen_bodies.clone(),
            charly: soldier.report_charly,
            charly_seen: soldier.report_charly != 0,
        };
        self.base.consider_report_merged(&soldier_report, 1 | 2 | 4); // BODIES | CHARLY | TYPE

        // Share our (now updated) report back to the soldier.
        // soldier.ConsiderReport(my_reconnaissance_report, 0)
        // — flags=0 means only update type, not bodies/charly.
        self.base
            .pending_cross_npc_actions
            .push(CrossNpcAction::UpdateReport {
                target: soldier_handle,
                report_type: self.base.my_reconnaissance_report.report_type,
                seek_position: self.base.my_reconnaissance_report.seek_position,
            });

        // Check if the report is really alerting
        if soldier_report_type > my_old_report_type
            && soldier_report_type > ReportType::Body
            && (!already_sent_out_soldiers || my_old_report_type == ReportType::MissedCharly)
        {
            // Alert!
            self.set_state(
                AiState::Seeking,
                Substate::SeekingOfficerGetAlertingReportFromSoldier,
            );
            self.base.antagonist = soldier_handle;
            self.face_npc(soldier_handle, tick);
            self.base.seek_position = soldier_seek_position;
            self.base
                .my_reconnaissance_report
                .update(soldier_report_type, soldier_seek_position);
            self.base
                .launch_timer(combat::STANDARD_TALK_TIME as u32, ctx.frame);
            return true;
        }

        false
    }

    // -----------------------------------------------------------------------
    // AlertOfficer — soldier alerts nearby officer
    // Mirrors the original officer search and "another soldier is
    // already alerting" suppression gate.
    // -----------------------------------------------------------------------

    pub fn alert_officer(
        &mut self,
        center: Position,
        _flags: u16,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        debug_assert_eq!(self.get_rank(), ProfileRank::Soldier);

        // Focus(NULL) — drop any prior gaze lock so the
        // soldier doesn't keep staring at the trigger entity while running
        // to the officer.
        self.base.pending_unfocus = true;

        self.base.alert_soldiers_point = center;

        let my_pos = ctx.position;
        let my_layer = ctx.position.level;

        // Special case: we were instructed to report back to an officer
        // after seeking (REPORT_OFFICER_AFTER flag).
        let mut nearest_officer: Option<&CampSoldierInfo> = None;

        if self.seek_flags.contains(SeekFlags::REPORT_OFFICER_AFTER) && self.base.antagonist != 0 {
            // Find the antagonist in camp_soldiers
            if let Some(ant) = tick
                .camp_soldiers
                .iter()
                .find(|cs| cs.handle == self.base.antagonist)
            {
                match ant.ai_substate {
                    Substate::SeekingOfficerWaitForInstructedSoldier => {
                        nearest_officer = Some(ant);
                    }
                    Substate::SeekingOfficerWaitForInstructedGroup => {
                        // Return to the officer
                        self.base.set_emoticon(EmoticonType::None);
                        self.go_near(
                            AiState::Seeking,
                            Substate::SeekingSoldierReturnToOfficer,
                            ant.position,
                            40,
                            GotoFlags::RUN,
                            ctx,
                        );
                        self.base.launch_timer(20, ctx.frame);
                        self.seek_flags &= !SeekFlags::REPORT_OFFICER_AFTER;
                        return true;
                    }
                    _ => {}
                }
            }
        }

        if nearest_officer.is_none() {
            let mut max_distance = combat::MAX_ALERT_OFFICER_RADIUS as f32;

            for cs in &tick.camp_soldiers {
                match cs.rank {
                    ProfileRank::Officer => {
                        // Candidate officer: must be able to fight, in DEFAULT
                        // state, and not script-locked.
                        if !cs.is_able_to_help
                            || cs.ai_state != AiState::Default
                            || cs.script_locked
                        {
                            continue;
                        }

                        // MaxNorm distance
                        let dx = (cs.position.x - my_pos.x).abs();
                        let dy = (cs.position.y - my_pos.y).abs();
                        let mut distance = dx.max(dy);

                        // Layer change penalty (only fires
                        // when the candidate officer is currently inside a
                        // building — `pSoldier->GetBuilding() != NULL`).
                        if cs.in_building && cs.layer != my_layer {
                            distance += parameters_ai::LAYER_CHANGE_PENALTY
                                * (my_layer as f32 - cs.layer as f32).abs();
                        }

                        if distance < max_distance {
                            max_distance = distance;
                            nearest_officer = Some(cs);
                        }
                    }
                    ProfileRank::Soldier => {
                        // Check if another soldier is already reporting to
                        // an officer — if so, don't duplicate the report.
                        match cs.ai_substate {
                            Substate::SeekingSoldierCalledByOfficer
                            | Substate::SeekingSoldierGoToOfficer
                            | Substate::SeekingSoldierGetInstructedByOfficer
                            | Substate::SeekingSoldierReturnToOfficer
                            | Substate::SeekingSoldierGiveReportToOfficer
                            | Substate::SeekingSoldierGiveAlertingReportToOfficerStart
                            | Substate::SeekingSoldierGiveAlertingReportToOfficerPoint
                            | Substate::SeekingSoldierGiveAlertingReportToOfficerEnd
                            | Substate::SeekingGroupCalledByOfficer
                            | Substate::SeekingGroupGoToOfficer
                            | Substate::SeekingGroupGetInstructedByOfficer
                            | Substate::SeekingRunningToOfficer
                            | Substate::SeekingRunningToOfficerSeen => {
                                if self.is_detecting_360_degrees(cs.handle, ctx) {
                                    // Another soldier is already alerting an
                                    // officer — abort.
                                    return false;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }

        let Some(officer) = nearest_officer else {
            // No officer found — clear flags and give up.
            self.seek_flags = SeekFlags::empty();
            return false;
        };

        // Alert this officer.
        let officer_handle = officer.handle;
        // nearest_officer.ForecastDestinationForIA(...)
        // — head to where the officer will be, not where they are now.
        let officer_target_pos = officer.forecast_destination;

        self.current_task_priority = task_priority::ALERT;
        self.base.antagonist = officer_handle;
        // Track the officer so the soldier can detect them on the way.
        self.base.pending_add_detectables.push((
            crate::element::EntityId::Soldier(crate::entity_id::SoldierId(officer_handle)),
            crate::element::DetectableType::Friend,
        ));
        self.gather_position = officer_target_pos;
        self.go_near(
            AiState::Seeking,
            Substate::SeekingRunningToOfficer,
            officer_target_pos,
            parameters_ai::AI_TALK_DISTANCE,
            GotoFlags::RUN,
            ctx,
        );
        self.base.launch_timer(50, ctx.frame);

        if self.base.couldnt_reachpoint {
            self.base.couldnt_reachpoint = false;
            return false;
        }

        true
    }

    // -----------------------------------------------------------------------
    // CreateListOfSoldiersYouCanAlert + GetNearestFighter + OfficerLookForSoldier
    // -----------------------------------------------------------------------

    /// Build the alert list for this NPC so they can later report to
    /// nearby friends. Populates the `DETECTABLE_FRIEND` list via
    /// `pending_*_detectables`, matching the rank-policy table:
    ///   - civilian    → all soldiers/officers/knights
    ///   - rank SOLDIER → officers only (and reset detected body unless `BODY`)
    ///   - rank OFFICER → simple soldiers only
    ///   - rank KNIGHT  → nothing (asserted away upstream)
    pub fn create_list_of_soldiers_you_can_alert(
        &mut self,
        position: Position,
        reason: ReportType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        self.base
            .pending_delete_detectables
            .push(crate::element::DetectableType::Friend);
        // Remember the alert point; clear body unless this is a
        // body-alert.
        self.base.alert_soldiers_point = position;
        if reason != ReportType::Body {
            self.base.detected_body = 0;
        }

        // Rank policy table.
        // Civilian mode is accepted via `self_is_soldier == false` — the
        // waypoint-macro executor keys off the same flag.
        let (allow_soldier, allow_officer, allow_knight) = if !ctx.self_is_soldier {
            (true, true, true)
        } else {
            match self.get_rank() {
                ProfileRank::Soldier => (false, true, false),
                ProfileRank::Officer => (true, false, false),
                // A knight should never run this.
                ProfileRank::Knight | ProfileRank::None => (false, false, false),
            }
        };

        // If our patrol's #0 is a matching rank, put them at the head
        // of the list. `camp_soldiers` only carries soldiers, so
        // civilian patrol chiefs don't qualify (the gate is on
        // is_soldier before reading the rank).
        let patrol_head: Option<NpcHandle> = self
            .base
            .patrol
            .first()
            .copied()
            .filter(|&h| h != 0)
            .and_then(|h| {
                tick.camp_soldiers.iter().find_map(|cs| {
                    if cs.handle != h {
                        return None;
                    }
                    let allowed = match cs.rank {
                        ProfileRank::Soldier => allow_soldier,
                        ProfileRank::Officer => allow_officer,
                        ProfileRank::Knight => allow_knight,
                        ProfileRank::None => false,
                    };
                    allowed.then_some(h)
                })
            });
        if let Some(h) = patrol_head {
            self.base.pending_add_detectables.push((
                crate::element::EntityId::Soldier(crate::entity_id::SoldierId(h)),
                crate::element::DetectableType::Friend,
            ));
        }

        // Fill the rest. `camp_soldiers` is already filtered to the
        // same camp.
        for cs in &tick.camp_soldiers {
            if cs.handle == 0 {
                continue;
            }
            if Some(cs.handle) == patrol_head {
                continue;
            }
            let allowed = match cs.rank {
                ProfileRank::Soldier => allow_soldier,
                ProfileRank::Officer => allow_officer,
                ProfileRank::Knight => allow_knight,
                ProfileRank::None => false,
            };
            if !allowed {
                continue;
            }
            self.base.pending_add_detectables.push((
                crate::element::EntityId::Soldier(crate::entity_id::SoldierId(cs.handle)),
                crate::element::DetectableType::Friend,
            ));
        }
    }

    /// Returns the handle of the nearest same-camp fighter matching
    /// `rank` + the `DefaultStateOrLookingBody` condition used by
    /// `OfficerLookForSoldier`. `camp_soldiers` only holds soldier NPCs
    /// — civilians are never candidates (the helper's only caller asks
    /// for `RANK_SOLDIER` specifically).
    fn get_nearest_fighter_default_or_looking_body(
        &self,
        my_pos: Position,
        max_radius: u16,
        rank: ProfileRank,
        tick: &AiPerTickData,
    ) -> Option<NpcHandle> {
        let max_sq = (max_radius as f32) * (max_radius as f32);
        let mut best: Option<(NpcHandle, f32)> = None;
        for cs in &tick.camp_soldiers {
            // Exclude self + dead/unconscious; require active.
            // `camp_soldiers` is already built from active, non-self
            // soldiers, and `is_able_to_help` covers alive+conscious
            // plus the narrower state/substate gates.
            if !cs.is_able_to_help {
                continue;
            }
            // Rank check (only set when caller passes a specific rank).
            if rank != ProfileRank::None && cs.rank != rank {
                continue;
            }
            // CONDITION_IS_IN_DEFAULT_STATE_OR_LOOKING_BODY:
            //   STATE_DEFAULT, or STATE_SEEKING +
            //   SUBSTATE_SEEKING_BODY_REACTIONTIME.
            let cond_ok = match cs.ai_state {
                AiState::Default => true,
                AiState::Seeking => cs.ai_substate == Substate::SeekingBodyReactiontime,
                _ => false,
            };
            if !cond_ok {
                continue;
            }

            let dx = cs.position.x - my_pos.x;
            let dy = (cs.position.y - my_pos.y) * INVERSE_ASPECT_RATIO;
            let sq = dx * dx + dy * dy;
            if sq > max_sq {
                continue;
            }
            if best.is_none_or(|(_, b)| sq <= b) {
                best = Some((cs.handle, sq));
            }
        }
        best.map(|(h, _)| h)
    }

    /// Officer looks for a nearby soldier to alert.
    pub fn officer_look_for_soldier(
        &mut self,
        reason: ReportType,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        debug_assert_eq!(self.get_rank(), ProfileRank::Officer);

        // Prefer the patrol's #0 if they're a plain
        // soldier.  `camp_soldiers` lets us recover the rank.
        let mut soldier: Option<(NpcHandle, Position)> = self
            .base
            .patrol
            .first()
            .copied()
            .filter(|&h| h != 0)
            .and_then(|h| {
                tick.camp_soldiers.iter().find_map(|cs| {
                    (cs.handle == h && cs.rank == ProfileRank::Soldier)
                        .then_some((cs.handle, cs.position))
                })
            });

        if soldier.is_none() {
            // GetNearestFighter(camp, 200,
            //   CONDITION_IS_IN_DEFAULT_STATE_OR_LOOKING_BODY,
            //   RANK_SOLDIER).
            if let Some(h) = self.get_nearest_fighter_default_or_looking_body(
                ctx.position,
                200,
                ProfileRank::Soldier,
                tick,
            ) && let Some(cs) = tick.camp_soldiers.iter().find(|cs| cs.handle == h)
            {
                soldier = Some((h, cs.position));
            }
        }

        // Seed the DETECTABLE_FRIEND list so later give-alerting-report
        // sequences can iterate the friends who need updates.
        self.create_list_of_soldiers_you_can_alert(self.base.seek_position, reason, ctx, tick);
        self.set_state(
            AiState::Seeking,
            Substate::SeekingOfficerLookingForSoldiers1,
        );

        if let Some((h, _)) = soldier {
            self.base.face_entity(h, ctx);
        } else {
            // FaceTo( (direction + 5) % 16 ).
            let new_dir = (ctx.direction + 5) % 16;
            self.base.face_direction(new_dir, ctx);
        }
        self.base.launch_timer(20, ctx.frame);
    }
    // -----------------------------------------------------------------------
    // Tower guard
    // -----------------------------------------------------------------------

    /// TowerGuardCallAlert.
    /// Broadcasts a tower-guard alert: every same-camp soldier within
    /// `SQR_TOWER_GUARD_ALERT_RADIUS` that isn't itself a tower guard,
    /// isn't holed up in a building, and is able to help gets a
    /// `CALL_TOWER_GUARD_ALERT` stimulus via the deferred inter-NPC
    /// Think queue.  The nearest reachable officer additionally gets a
    /// `CALL_TOWER_GUARD_CALLS_ME` so they come to investigate.  If no
    /// officer is in ear-shot but a "far officer" exists, the nearest
    /// hearing soldier is tasked to run to that officer instead.
    pub fn tower_guard_call_alert(
        &mut self,
        danger_pos: Position,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        use crate::profiles::ProfileRank;
        debug_assert!(self.tower_guard);

        // Assert tower guard, write seek position,
        // build alert hint, Say(REMARK_CRY_ALERT).  `friends_are_alerted`
        // and the reconnaissance report are NOT touched here — they
        // are set by the decision-dispatch caller (Decision::TowerGuardAlert
        // at L12593) before the SetState→TowerGuardCallAlert flow.
        self.base.seek_position = danger_pos;
        self.base.say(Remark::CryAlert);

        let my_camp = ctx.camp;
        let my_pos = ctx.position;
        let alert_hint = Hint {
            seek_point: danger_pos,
            seek_flags: 0,
            who_tells_me: self.base.me,
        };

        // Two categorisations emerge from the loop:
        //   1. Soldiers *inside* SQR_TOWER_GUARD_ALERT_RADIUS who
        //      can hear the alert directly.
        //   2. Officers *outside* the radius who can be reached via a
        //      runner (nearest far officer, picked by distance).
        let mut in_range_soldiers: Vec<crate::ai::NpcHandle> = Vec::new();
        let mut nearest_officer: Option<(crate::ai::NpcHandle, f32)> = None;
        let mut nearest_far_officer: Option<(crate::ai::NpcHandle, f32, Position)> = None;
        let sqr_radius = combat::SQR_TOWER_GUARD_ALERT_RADIUS as f32;

        for (&handle, view) in ctx.entity_views.iter() {
            if handle == self.base.me {
                continue;
            }
            if !view.is_soldier() || view.camp != my_camp {
                continue;
            }
            // Uses `is_able_to_help`, not `is_able_to_fight` —
            // soldiers in STATE_ATTACKING / MENACING / FLEEING /
            // SLEEPING are excluded so an active swordfighter is not
            // interrupted by a tower-guard cry.
            if !soldier_is_able_to_help_state(
                view.is_able_to_fight,
                view.ai_state,
                view.ai_substate,
            ) {
                continue;
            }
            if view.is_tower_guard {
                continue;
            }
            if view.in_building {
                continue;
            }
            // SquareDistance stretches Y by `INVERSE_ASPECT_RATIO`.
            let dx = view.position.x - my_pos.x;
            let dy = (view.position.y - my_pos.y) * INVERSE_ASPECT_RATIO;
            let sq_dist = dx * dx + dy * dy;

            if sq_dist < sqr_radius {
                // This soldier hears the cry. Rank
                // classification happens in the second pass using
                // the per-tick camp-soldier snapshot, which does
                // carry rank.
                in_range_soldiers.push(handle);
            } else if view.rank == ProfileRank::Officer
                && sq_dist
                    < nearest_far_officer
                        .map(|(_, d, _)| d)
                        .unwrap_or(f32::INFINITY)
            {
                // Only consider RANK_OFFICER for the
                // far-officer fallback.  AiEntityView carries `rank`
                // so we can apply the same gate here.
                nearest_far_officer = Some((handle, sq_dist, view.position));
            }
        }

        // Queue the in-range alerts. Dispatch these
        // synchronously in-loop, but the Rust engine's deferred
        // cross-NPC action pass delivers them later in the same
        // tick — same observable ordering for the target.
        for handle in &in_range_soldiers {
            self.base
                .pending_cross_npc_actions
                .push(CrossNpcAction::SendStimulus {
                    target: *handle,
                    stimulus_type: StimulusType::CallTowerGuardAlert,
                    info: StimulusInfo::Hint(alert_hint),
                    fallback_to_sender: None,
                    to_whole_patrol: false,
                });
        }

        // Pick the closest in-range officer: use the per-tick camp
        // soldier snapshot, which does carry rank. Filter
        // on `pSoldier->GetRank() == RANK_OFFICER`.
        for cs in tick.camp_soldiers.iter() {
            if cs.rank != ProfileRank::Officer {
                continue;
            }
            if !in_range_soldiers.contains(&cs.handle) {
                continue;
            }
            let dx = cs.position.x - my_pos.x;
            let dy = cs.position.y - my_pos.y;
            let sq_dist = dx * dx + dy * dy;
            if nearest_officer.is_none_or(|(_, d)| sq_dist < d) {
                nearest_officer = Some((cs.handle, sq_dist));
            }
        }

        if let Some((officer, _)) = nearest_officer {
            // Directly alert the officer — they'll come
            // investigate via `CallTowerGuardCallsMeStandardProcedure`.
            self.base
                .pending_cross_npc_actions
                .push(CrossNpcAction::SendStimulus {
                    target: officer,
                    stimulus_type: StimulusType::CallTowerGuardCallsMe,
                    info: StimulusInfo::Hint(alert_hint),
                    fallback_to_sender: None,
                    to_whole_patrol: false,
                });
            return;
        }

        // No in-range officer — look for a runner. Pick
        // the in-range soldier that is closest to the nearest
        // out-of-range officer, and have them run to that officer.
        //
        // Note: the reference contains a long-standing bug —
        // it iterates `i < listSimpleSoldiersWhoHearMe.Size()` but
        // dereferences `GetSoldier(camp, i)` rather than
        // `listSimpleSoldiersWhoHearMe[i]`, picking the first N
        // soldiers of the camp roster instead of the actual hearing
        // list.  We do *not* replicate the bug — the Rust port walks
        // `in_range_soldiers` and picks the closest hearer to the
        // far officer.
        let Some((_, _, officer_pos)) = nearest_far_officer else {
            return;
        };
        let mut runner: Option<(crate::ai::NpcHandle, f32)> = None;
        for handle in &in_range_soldiers {
            let Some(view) = ctx.entity_view(*handle) else {
                continue;
            };
            let dx = view.position.x - officer_pos.x;
            let dy = (view.position.y - officer_pos.y) * INVERSE_ASPECT_RATIO;
            let sq_dist = dx * dx + dy * dy;
            if runner.is_none_or(|(_, d)| sq_dist < d) {
                runner = Some((*handle, sq_dist));
            }
        }

        if let Some((runner_handle, _)) = runner {
            self.base
                .pending_cross_npc_actions
                .push(CrossNpcAction::SendStimulus {
                    target: runner_handle,
                    stimulus_type: StimulusType::CallTowerGuardCallsMe,
                    info: StimulusInfo::Hint(alert_hint),
                    fallback_to_sender: None,
                    to_whole_patrol: false,
                });
        }
    }

    // -----------------------------------------------------------------------
    // RunAndAlertSoldiers — officer flees to a door with 3+ reservists
    // -----------------------------------------------------------------------
    //
    // Searches building doors for the one whose weighted distance
    // (`MaxNorm(door.point_out - me) /
    // reservists_behind`) is minimal, with a +1000 malus for layer
    // changes, and runs to its entry point with
    // `SUBSTATE_FLEEING_RUN_TO_ALERT_SOLDIERS`.  Returns `true` on a
    // match, `false` if no qualifying door exists.

    pub fn run_and_alert_soldiers(
        &mut self,
        center: Position,
        ctx: &AiContext,
        tick: &AiPerTickData,
        global: &AiGlobalState,
    ) -> bool {
        use crate::profiles::ProfileRank;

        // Focus(NULL) — clear focus target.
        self.base.pending_unfocus = true;

        self.base.seek_position = center;

        let my_pos = ctx.position;
        let my_layer = ctx.position.level;

        let mut min_weighted: f32 = f32::INFINITY;
        let mut best_door: Option<&crate::ai::DoorSeekInfo> = None;

        for door in &global.door_seek_infos {
            // Only building doors count.
            if !matches!(door.door_type, crate::gate::DoorType::Building) {
                continue;
            }
            // door.IsActorAutorized(true, me, false).
            if !door.npc_villain_authorized_direct {
                continue;
            }

            // NumberOfReservistsBehindDoor: count same-camp
            // rank-SOLDIER, able-to-help occupants of the building
            // whose sector matches `door.sector_in`.  We cross-reference
            // `tick.camp_soldiers` (has rank) against the entity view
            // map (has `in_building` + `building_sector`) to reproduce
            // the building.GetOccupant walk.
            let reservists = tick
                .camp_soldiers
                .iter()
                .filter(|cs| cs.rank == ProfileRank::Soldier && cs.is_able_to_help)
                .filter(|cs| {
                    ctx.entity_view(cs.handle)
                        .map(|v| {
                            v.in_building
                                && v.building_sector.map(u16::from) == Some(door.sector_in)
                        })
                        .unwrap_or(false)
                })
                .count();

            if reservists < 3 {
                continue;
            }

            // (door.point_out - me).MaxNorm().
            let dx = (door.point_out.x - my_pos.x).abs();
            let dy = (door.point_out.y - my_pos.y).abs();
            let mut weighted = dx.max(dy);

            // +1000 malus when the door is on a different layer.
            if door.layer_out != my_layer {
                weighted += 1000.0;
            }

            // Divide by reservist count — more reservists = better door.
            weighted /= reservists as f32;

            if weighted < min_weighted {
                min_weighted = weighted;
                best_door = Some(door);
            }
        }

        let Some(door) = best_door else {
            return false;
        };

        // my_door = nearest_door — stash the door so the
        // subsequent indoor `AlertSoldiers` formation flow has the
        // right exit-door geometry to project gather slots outside.
        self.my_door_index = Some(door.door_index.0);
        self.go_to(
            AiState::Fleeing,
            Substate::FleeingRunToAlertSoldiers,
            door.position_in,
            GotoFlags::RUN,
            ctx,
        );
        true
    }
}
