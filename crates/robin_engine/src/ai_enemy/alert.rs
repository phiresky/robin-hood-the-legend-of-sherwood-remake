//! Officer/soldier alert coordination: `alert_soldiers`,
//! `alert_officer`, `tower_guard_call_alert`, `run_and_alert_soldiers`,
//! `command_soldiers_to_attack`, `officer_look_for_soldier`, the
//! soldier-formation layout helper `can_put_soldiers_in_this_direction`,
//! the friend-list builder `create_list_of_soldiers_you_can_alert`,
//! and the report-merging helper `get_report_from_soldier`.

use crate::ai::*;
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::parameters_ai;
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO};

use super::util::{ai_max_norm_distance, iso_normalize, vec_to_sector, vec_to_sector_ar};
use super::{CampSoldierInfo, EnemyAi, ProfileRank, SeekFlags, combat, task_priority};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandSoldiersStart {
    Pending,
    Rejected,
}

/// Direction seed used by Original's officer formation sweeps.
///
/// `AlertSoldiers` and `CommandSoldiersToAttack` both call
/// `SBGeoVector2D::GetSector0to15(ASPECT_RATIO)` here. Keep the aspect
/// argument explicit: using the aspect-1 classifier changes diagonal map-space
/// vectors by a sector.
fn formation_direction(dx: f32, dy: f32) -> u16 {
    vec_to_sector_ar(dx, dy, ASPECT_RATIO)
}

/// Return Original's raw formation-loop cursor and its projected sector.
///
/// `RHArtificialMalignity::AlertSoldiers` increments `uwDirection` without
/// wrapping and masks only the argument passed to
/// `CanPutSoldiersInThisDirection`.  After a successful attempt it stores the
/// raw cursor (XORed with 8) in each soldier's `muwGatherDirection`.  Values
/// such as 29 therefore deliberately survive even though actor direction 29
/// projects to sector 13.  Preserving those high bits matters to `FaceTo`'s
/// raw equality check: 29 is not already-facing 13, so Original still authors
/// a Turn.
fn formation_sweep_cursor(average_direction: u16, offset: u16) -> (u16, u16) {
    let raw = average_direction + offset;
    (raw, raw & 15)
}

/// Sum the unit directions used by Original's officer attack broadcast.
///
/// `SBGeoVector2D::GetNormalized()` divides unconditionally in shipping
/// builds. An alerted soldier standing exactly on the officer therefore adds
/// `(NaN, NaN)` rather than being skipped; the later sector classifier maps
/// that poisoned accumulator to sector 0 because every comparison is false.
fn average_alerted_direction_vector(
    officer: Position,
    alerted_positions: &[Position],
) -> (f32, f32) {
    alerted_positions
        .iter()
        .fold((0.0, 0.0), |(sum_x, sum_y), soldier_pos| {
            let direction =
                iso_normalize((soldier_pos.x - officer.x, soldier_pos.y - officer.y), 1.0);
            (sum_x + direction.0, sum_y + direction.1)
        })
}

/// Direction used by Original's officer attack-point sequence:
/// `PositionToPoint3D(target) - officer.GetPosition()`.
fn attack_point_direction(ctx: &AiContext, target: Position) -> u16 {
    let target_world = ctx.position_to_point_3d(target);
    let dx = target_world.x - ctx.position.x;
    let dy = target_world.y - (ctx.position.y + ctx.elevation);
    vec_to_sector(dx, dy)
}

/// Original stores the stretched squared norm in an `ULONG` before every
/// comparison in `TowerGuardCallAlert`. Preserve that truncation here: two
/// fractional distances in the same integer bucket compare equal.
fn tower_guard_square_distance(a: WorldPoint3D, b: WorldPoint3D) -> u32 {
    let dx = a.x - b.x;
    let dy = (a.y - b.y) * INVERSE_ASPECT_RATIO;
    let dz = a.z - b.z;
    (dx * dx + dy * dy + dz * dz) as u32
}

fn alert_officer_distance(
    officer: &Position,
    officer_elevation: f32,
    officer_layer: u16,
    officer_in_building: bool,
    owner: &Position,
    owner_elevation: f32,
    owner_layer: u16,
) -> u32 {
    let mut distance =
        ai_max_norm_distance(officer, officer_elevation, owner, owner_elevation) as u32;
    if officer_in_building && officer_layer != owner_layer {
        distance += (parameters_ai::LAYER_CHANGE_PENALTY
            * (owner_layer as f32 - officer_layer as f32).abs()) as u32;
    }
    distance
}

/// Original `AlertOfficer` admits officers through `IsAbleToFight`, not the
/// distinct `IsAbleToHelp` predicate used by several officer-coordination
/// scans. In particular, an inactive Default-state officer can help according
/// to the latter but cannot be selected as somebody to alert.
fn can_alert_officer(
    rank: ProfileRank,
    is_able_to_fight: bool,
    state: AiState,
    script_locked: bool,
) -> bool {
    rank == ProfileRank::Officer && is_able_to_fight && state == AiState::Default && !script_locked
}

/// Substates for which Original's `AlertOfficer` scan treats a soldier as
/// already reporting to an officer.
///
/// The global camp registry includes the caller itself. Our
/// `tick.camp_soldiers` snapshot deliberately omits the owner, so callers
/// must splice the owner back into its stable handle-order slot.
fn is_alerting_an_officer(substate: Substate) -> bool {
    matches!(
        substate,
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
            | Substate::SeekingRunningToOfficerSeen
    )
}

fn sorted_owner_insertion_index<T>(
    entries: &[T],
    owner: NpcHandle,
    handle: impl Fn(&T) -> NpcHandle,
) -> usize {
    entries.partition_point(|entry| handle(entry) < owner)
}

/// Distance stored on each accepted soldier before Original inserts it into
/// the officer's farthest-first alert list. `SquareDistance` reads literal 3D
/// element positions, so projected map Y alone is insufficient on ramps.
fn alert_soldier_sort_distance(
    soldier_position: WorldPoint3D,
    officer_position: WorldPoint3D,
) -> f32 {
    let dx = soldier_position.x - officer_position.x;
    let dy = (soldier_position.y - officer_position.y) * INVERSE_ASPECT_RATIO;
    let dz = soldier_position.z - officer_position.z;
    dx * dx + dy * dy + dz * dz
}

/// `Q_SHALL_I_STAY_ON_MY_POST` branch selection.
///
/// Original tests alcohol first. A sufficiently drunk soldier stays on post
/// even while inactive or indoors. It enters the normal outdoor answers only when
/// `IsActiveAndOutsideBuilding()` is true. Inactive soldiers therefore use
/// the indoor answer (`false`) and are allowed to leave even when their
/// profile marks them as duty soldiers.
fn alert_soldier_stays_on_post(
    active: bool,
    in_building: bool,
    is_tower_guard: bool,
    duty_flag: bool,
    company_number: u16,
    blood_alcohol: u8,
) -> bool {
    i32::from(blood_alcohol) > crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT
        || (active && !in_building && (is_tower_guard || duty_flag || company_number == 100))
}

/// Preserve `AlertSoldiers`' two positive, strict radius predicates.
///
/// Original first requires `MaxNorm(delta) < ALERT_RADIUS`, then requires
/// `SquareNorm(delta) < ALERT_RADIUS * ALERT_RADIUS`.  Expressing either test
/// as an inverted `>=` rejection is not equivalent for unordered floats: a
/// legacy NaN position fails Original's positive comparison and must not be
/// admitted to the alert broadcast.
fn alert_soldier_is_inside_radius(candidate: Position, officer: Position, radius: f32) -> bool {
    let dx = (candidate.x - officer.x).abs();
    let dy = (candidate.y - officer.y).abs();
    dx.max(dy) < radius && dx * dx + dy * dy < radius * radius
}

fn sort_alerted_soldiers(alerted: &mut [(HumanHandle, f32, usize)]) {
    alerted.sort_by(
        |(_, lhs_distance, lhs_index), (_, rhs_distance, rhs_index)| {
            rhs_distance
                .total_cmp(lhs_distance)
                .then_with(|| rhs_index.cmp(lhs_index))
        },
    );
}

/// `camp_soldiers` intentionally excludes the evaluating NPC for its normal
/// consumers. The fallback bug in Original's `TowerGuardCallAlert`, however,
/// dereferences the complete camp registry (including the tower guard) for its
/// first-N scan. Entity handles follow that registry's stable slot order, so
/// insert the owner back at its handle position before reproducing the bug.
fn tower_guard_complete_registry(
    camp_registry_without_owner: impl IntoIterator<Item = (NpcHandle, WorldPoint3D)>,
    owner: (NpcHandle, WorldPoint3D),
) -> Vec<(NpcHandle, WorldPoint3D)> {
    let mut registry: Vec<_> = camp_registry_without_owner.into_iter().collect();
    let owner_index = registry.partition_point(|(handle, _)| *handle < owner.0);
    assert!(
        registry
            .get(owner_index)
            .is_none_or(|entry| entry.0 != owner.0),
        "tower-guard owner {} is already present in the self-excluding camp registry",
        owner.0
    );
    registry.insert(owner_index, owner);
    registry
}

fn tower_guard_runner_from_registry_prefix(
    camp_registry: impl IntoIterator<Item = (NpcHandle, WorldPoint3D)>,
    hearing_simple_soldier_count: usize,
    officer_position: WorldPoint3D,
    officer_distance_from_tower_squared: u32,
) -> Option<NpcHandle> {
    let mut runner = None;
    for (handle, position) in camp_registry.into_iter().take(hearing_simple_soldier_count) {
        let square_distance = tower_guard_square_distance(position, officer_position);
        if square_distance < officer_distance_from_tower_squared {
            runner = Some(handle);
        }
    }
    runner
}

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

    pub(crate) fn command_soldiers_to_attack(
        &mut self,
        center: Position,
        _global: &AiGlobalState,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> CommandSoldiersStart {
        debug_assert_eq!(self.get_rank(), ProfileRank::Officer);

        let my_pos = ctx.position;
        self.base.seek_position = center;
        self.current_task_priority = task_priority::ALERT;

        let alert_radius = combat::ALERT_RADIUS as f32;
        let alert_radius_sq = alert_radius * alert_radius;

        self.alerted_us.clear();
        self.pending_alert_soldier_candidates.clear();
        let mut last_result_request = None;

        // `alert_soldier_candidates` already carries Original's rank and
        // IsAbleToFight gates, over every camp's NPCs in registry order;
        // the recipient's live Think handles script/AI locks and state gates.
        for cs in &tick.alert_soldier_candidates {
            // Original calls `pFriend->IsDetecting360Degrees(mpMe)` here,
            // after the cheap rank/body gates and before its distance gates.
            // Evaluating this while constructing every tick snapshot changes
            // the observable LOS call stream even when no officer broadcasts.
            if !super::soldier_detects_target_360(
                cs.position,
                cs.elevation,
                cs.is_rider,
                cs.view_radius,
                cs.in_building,
                ctx.position,
                ctx.elevation,
                ctx.posture,
                ctx.self_is_rider,
                ctx.direction as i16,
                ctx.in_building,
                ctx.obstacle_list(),
            ) {
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
            last_result_request = Some(self.base.outbox.reentrant.cross_npc_actions.len());
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::RequestThinkResult {
                    target: cs.handle,
                    caller: self.base.me,
                    stimulus_type: StimulusType::CallCombatAlert,
                    info: StimulusInfo::Position(center),
                    continuation: ThinkResultContinuation::OfficerCombatAlertedSoldier {
                        last: false,
                        use_formation: grid.is_some(),
                    },
                });
        }

        if let Some(index) = last_result_request
            && let CrossNpcAction::RequestThinkResult { continuation, .. } =
                &mut self.base.outbox.reentrant.cross_npc_actions[index]
        {
            *continuation = ThinkResultContinuation::OfficerCombatAlertedSoldier {
                last: true,
                use_formation: grid.is_some(),
            };
        }

        if last_result_request.is_some() {
            CommandSoldiersStart::Pending
        } else {
            CommandSoldiersStart::Rejected
        }
    }

    pub(super) fn finish_command_soldiers_to_attack(
        &mut self,
        _global: &AiGlobalState,
        _grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        _tick: &AiPerTickData,
    ) -> bool {
        let center = self.base.seek_position;
        let my_pos = ctx.position;
        let alerted_count = self.alerted_us.len() as u16;
        if alerted_count > 0 {
            let alerted_positions: Vec<Position> = self
                .alerted_us
                .iter()
                .map(|handle| {
                    // `CommandSoldiersToAttack` alerts rank soldiers of any
                    // camp, so the recipients are not all in the same-camp
                    // roster; Original reads `Point(pFriend)` off the actor.
                    ctx.entity_view(*handle)
                        .unwrap_or_else(|| {
                            panic!(
                                "combat-alerted soldier {} disappeared from officer {} entity views",
                                handle, self.base.me
                            )
                        })
                        .position
                })
                .collect();
            let (avg_dir_vec_x, avg_dir_vec_y) =
                average_alerted_direction_vector(my_pos, &alerted_positions);

            self.base.stop_all();

            // Build the turn/gather/point sequence.  If the enemy
            // (`center`) is further than 150 units (MaxNorm) from the
            // officer, first turn toward the average soldier direction
            // and gather them; then always point to the target.
            use crate::element::Command;
            use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

            let me_to_target_x = center.x - my_pos.x;
            let me_to_target_y = center.y - my_pos.y;

            // Original computes the pointing direction from
            // `PositionToPoint3D(mposSeekPosition) - mpMe->GetPosition()`.
            // `center` and `my_pos` are projected map positions, so their raw
            // Y delta is only suitable for the nearby-enemy MaxNorm gate
            // below.  Reconstruct world Y for the facing vector; otherwise an
            // officer standing above or below the target points several
            // sectors too far north/south.
            let target_dir = attack_point_direction(ctx, center);

            let owner = self.base.owner_entity_id;
            let mut seq = Sequence::new();
            let mut level: u16 = 1;

            let enemy_max_norm = me_to_target_x.abs().max(me_to_target_y.abs());
            if enemy_max_norm > 150.0 {
                // Turn to the soldiers (face average direction).
                let avg_dir = formation_direction(avg_dir_vec_x, avg_dir_vec_y);
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

            self.base.outbox.actor.launch_sequences.push(seq);

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
        _global: &AiGlobalState,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        tick: &AiPerTickData,
        failure: AlertSoldiersFailureContinuation,
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
        self.base.outbox.actor.set_unfocus();

        self.current_task_priority = task_priority::ALERT;

        // Reset the alerted / staying / us lists.
        self.alerted_us.clear();
        self.pending_alert_soldier_candidates.clear();
        self.base.list_alerted_us.clear();
        self.base.list_staying_us.clear();
        self.base.list_us.clear();

        debug_assert_eq!(self.get_rank(), ProfileRank::Officer);

        let alert_radius = combat::ALERT_RADIUS as f32;

        let my_handle = self.base.me;
        let mut candidates = Vec::new();

        for cs in &tick.camp_soldiers {
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
            // company-100 soldiers. Its drunk answer returns true before
            // selecting the normal active/outdoor or indoor branch; otherwise
            // the indoor answer is false, so those soldiers may leave.
            // After the drunk branch, Original's AnswerQuestion checks
            // IsActiveAndOutsideBuilding(). Inactive soldiers are absent
            // from `entity_views`, but that must select the indoor answer,
            // not be treated as active and outdoors.
            let stays_on_post = alert_soldier_stays_on_post(
                cs.active,
                cs.in_building,
                cs.is_tower_guard,
                cs.duty_flag,
                cs.company_number,
                cs.blood_alcohol,
            );
            let allowed_to_leave = !stays_on_post;
            let patrol_chief_is_me = cs
                .patrol_chief
                .is_some_and(|chief_id| chief_id.index() == my_handle);
            if !(allowed_to_leave || patrol_chief_is_me) {
                continue;
            }

            // CanCallThisSoldier — reject soldiers whose
            // patrol chief is someone else (within 700 units) or who
            // are already in a conversation with another antagonist.
            if !self.can_call_this_soldier(cs, ctx, tick) {
                continue;
            }

            // MaxNorm + SquareNorm radius gates. Original evaluates both
            // positions through RHArtificialIntelligence::Position. During a
            // door pass that resolves the candidate to its selected gate
            // endpoint, rather than the raw body position retained in
            // CampSoldierInfo for other consumers.
            let candidate_position = ctx
                .entity_view(cs.handle)
                .unwrap_or_else(|| {
                    panic!(
                        "AlertSoldiers candidate {} lacks the required AI entity view",
                        cs.handle
                    )
                })
                .position;
            // Keep Original's positive strict comparisons so unordered legacy
            // coordinates are rejected.
            if !alert_soldier_is_inside_radius(candidate_position, my_pos, alert_radius) {
                continue;
            }

            candidates.push(cs.handle);
        }

        if candidates.is_empty() {
            return false;
        }
        let first = candidates.remove(0);
        let first_is_last = candidates.is_empty();
        self.pending_alert_soldier_candidates = candidates;
        self.base
            .outbox
            .reentrant
            .cross_npc_actions
            .push(CrossNpcAction::RequestThinkResult {
                target: first,
                caller: self.base.me,
                stimulus_type: StimulusType::CallAlert,
                info: StimulusInfo::Human(self.base.me),
                continuation: ThinkResultContinuation::OfficerAlertedSoldier {
                    last: first_is_last,
                    use_formation: grid.is_some(),
                    failure,
                },
            });
        true
    }

    pub(super) fn finish_alert_soldiers(
        &mut self,
        global: &AiGlobalState,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        let my_pos = ctx.position;
        let my_world_pos = ctx
            .entity_view(self.base.me)
            .unwrap_or_else(|| {
                panic!(
                    "officer {} finalizing accepted alerts is missing its live entity view",
                    self.base.me
                )
            })
            .detection_position_world;
        let officer_in_building = ctx.in_building;

        // Preserve engine/acceptance order for the average-direction sum.
        // Original accumulates it while accepting soldiers, before inserting
        // them into its distance-sorted list.
        let accepted_handles = self.alerted_us.clone();
        let mut alerted: Vec<(HumanHandle, f32, usize)> = accepted_handles
            .iter()
            .enumerate()
            .map(|(acceptance_index, handle)| {
                let soldier = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == *handle)
                    .unwrap_or_else(|| {
                        panic!(
                            "alerted soldier {} disappeared from officer {} tick roster",
                            handle, self.base.me
                        )
                    });
                let distance = alert_soldier_sort_distance(soldier.position_world, my_world_pos);
                (*handle, distance, acceptance_index)
            })
            .collect();
        // The C++ comment claims increasing distance, but the insertion loop
        // advances while `new_distance < existing_distance`: farthest first.
        // Equal-distance newcomers are inserted before older entries.
        sort_alerted_soldiers(&mut alerted);
        self.alerted_us = alerted.iter().map(|(handle, _, _)| *handle).collect();

        let (avg_dir_vec_x, avg_dir_vec_y) = if officer_in_building {
            (0.0, 0.0)
        } else {
            accepted_handles
                .iter()
                .fold((0.0, 0.0), |(sum_x, sum_y), handle| {
                    let soldier = tick
                        .camp_soldiers
                        .iter()
                        .find(|cs| cs.handle == *handle)
                        .expect("accepted alert soldier was validated above");
                    let dx = soldier.position.x - my_pos.x;
                    let dy = soldier.position.y - my_pos.y;
                    let len = (dx * dx + dy * dy).sqrt();
                    if len > 0.0 {
                        (sum_x + dx / len, sum_y + dy / len)
                    } else {
                        (sum_x, sum_y)
                    }
                })
        };

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
            let avg_dir = formation_direction(vdx, vdy);
            (avg_dir, Some((door, step)))
        } else {
            (formation_direction(avg_dir_vec_x, avg_dir_vec_y), None)
        };

        // Try directions / door-step positions
        // until `CanPutSoldiersInThisDirection` succeeds.
        let mut chosen_slots: Option<Vec<Position>> = None;
        let mut chosen_direction_raw: u16 = avg_dir_start;
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
                        let (try_direction_raw, try_dir) =
                            formation_sweep_cursor(avg_dir_start, offset);
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
                            chosen_direction_raw = try_direction_raw;
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
                    let (try_direction_raw, try_dir) =
                        formation_sweep_cursor(avg_dir_start, offset);
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
                        chosen_direction_raw = try_direction_raw;
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
            let face_threat = chosen_direction_raw ^ 8;
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
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::InstructGatherPosition {
                            target: handle,
                            position: chosen,
                            direction: face_threat,
                            call_instruction: false,
                        },
                    );
                } else {
                    // Outdoor nearest-slot match.
                    let soldier_pos = tick
                        .camp_soldiers
                        .iter()
                        .find(|cs| cs.handle == handle)
                        .map(|cs| cs.position)
                        .unwrap_or_else(|| {
                            panic!(
                                "alerted soldier {} disappeared from officer {} tick roster",
                                handle, self.base.me
                            )
                        });
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
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::InstructGatherPosition {
                            target: handle,
                            position: chosen,
                            direction: face_threat,
                            call_instruction: false,
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

            // The officer turns toward the formation (i.e. toward the
            // soldiers) before the GatherSoldiers animation.  A
            // successful placement stores `match_dir ^ 8` as the gather
            // direction and the turn re-XORs it, so the turn lands on
            // `match_dir` — the raw successful loop cursor verbatim. Actor
            // direction projection masks it later; do not normalize it here.
            //
            // When no placement is found the gather direction keeps the
            // value the 16-direction sweep left behind, `avg_dir + 16`,
            // which never took the `^ 8` correction. The turn's own
            // `^ 8` then still applies, and the surplus 16 falls out in
            // the 16-sector wrap, leaving `avg_dir ^ 8` — the officer
            // faces *away* from the average soldier direction.
            let turn_dir = if placement_ok {
                chosen_direction_raw
            } else {
                avg_dir_start ^ 8
            };
            let owner = self.base.owner_entity_id;
            let mut seq = Sequence::new();

            // Turn toward the soldiers / threat.
            let mut turn_elem = SequenceElement::new_generic(1, Command::Turn, owner);
            turn_elem.set_property(Field::Direction, FieldValue::Integer(turn_dir as u32));
            seq.append_element(turn_elem);

            // Gather the soldiers.
            seq.append_element(SequenceElement::new(2, Command::GatherSoldiers, owner));

            self.base.outbox.actor.launch_sequences.push(seq);

            self.base.say(Remark::OfficerCallsGroup);

            self.base
                .set_transient_emoticon(EmoticonType::XMark, 20, ctx.frame);
            self.set_state(AiState::Seeking, Substate::SeekingOfficerWaitForGroup);
            self.base.launch_timer(20, ctx.frame);
        } else if placement_ok {
            // Indoor alert, found a free spot outside.
            // Stash the gather destination for the leave-house
            // substate transition.
            // After Original's first XOR, `uwDirection` is the raw successful
            // cursor XOR 8. The indoor assignment XORs it again, preserving
            // the raw cursor rather than its 0..15 projection.
            self.gather_position = chosen_officer_position;
            self.gather_direction = chosen_direction_raw;
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
            charly_seen: soldier.report_charly.is_some(),
        };
        self.base.consider_report_merged_at_frame(
            &soldier_report,
            1 | 2 | 4,
            ctx.entity_views.as_ref(),
            ctx.frame,
        ); // BODIES | CHARLY | TYPE

        // Share our (now updated) report back to the soldier. Original calls
        // `soldier.ConsiderReport(my_reconnaissance_report, 0)`: flags zero
        // deliberately leaves the soldier's stored report unchanged, but the
        // ConsiderReport body walk still removes every newly known body from
        // the soldier's BODY detectables.
        self.base
            .outbox
            .reentrant
            .cross_npc_actions
            .push(CrossNpcAction::ConsiderReport {
                target: soldier_handle,
                report: self.base.my_reconnaissance_report.clone(),
                flags: 0,
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
            self.base.antagonist = Some(AiEntityHandle::new(soldier_handle));
            self.face_npc(soldier_handle, ctx);
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
        sim: &crate::sim_rng::SimulationContext,
        _center: Position,
        _flags: u16,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        debug_assert_eq!(self.get_rank(), ProfileRank::Soldier);

        // Focus(NULL) — drop any prior gaze lock so the
        // soldier doesn't keep staring at the trigger entity while running
        // to the officer.
        self.base.outbox.actor.set_unfocus();

        let my_pos = ctx.position;
        let my_layer = ctx.position.level;

        // Special case: we were instructed to report back to an officer
        // after seeking (REPORT_OFFICER_AFTER flag).
        let mut nearest_officer: Option<&CampSoldierInfo> = None;

        if self.seek_flags.contains(SeekFlags::REPORT_OFFICER_AFTER)
            && self.base.antagonist.is_some()
        {
            // Find the antagonist in camp_soldiers
            if let Some(ant) = tick.camp_soldiers.iter().find(|cs| {
                self.base
                    .antagonist
                    .is_some_and(|handle| handle.get() == cs.handle)
            }) {
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
            let mut max_distance = combat::MAX_ALERT_OFFICER_RADIUS as u32;
            // Original walks the complete same-camp registry in stable handle
            // order. The tick snapshot omits the owner, so splice its literal
            // self-detection check back into that order instead of checking it
            // first. An earlier reporting friend must win and publish that
            // friend's visibility query before the scan reaches `mpMe`.
            let owner_index =
                sorted_owner_insertion_index(&tick.camp_soldiers, self.base.me, |soldier| {
                    soldier.handle
                });
            for registry_index in 0..=tick.camp_soldiers.len() {
                if registry_index == owner_index {
                    if is_alerting_an_officer(self.base.current_substate)
                        && self.is_detecting_360_degrees(self.base.me, ctx)
                    {
                        return false;
                    }
                    continue;
                }
                let snapshot_index = if registry_index < owner_index {
                    registry_index
                } else {
                    registry_index - 1
                };
                let cs = &tick.camp_soldiers[snapshot_index];
                match cs.rank {
                    ProfileRank::Officer => {
                        // Candidate officer: must be able to fight, in DEFAULT
                        // state, and not script-locked.
                        if !can_alert_officer(
                            cs.rank,
                            cs.is_able_to_fight,
                            cs.ai_state,
                            cs.script_locked,
                        ) {
                            continue;
                        }

                        // Original stores `MaxNormDistance` in an ULONG:
                        // compare the stretched 3D world-space Chebyshev
                        // distance after truncation. In particular, world Y
                        // is map Y plus elevation; comparing raw map Y can
                        // select a different officer across level layers.
                        let distance = alert_officer_distance(
                            &cs.position,
                            cs.position_world.z,
                            cs.layer,
                            cs.in_building,
                            &my_pos,
                            ctx.elevation,
                            my_layer,
                        );

                        if distance < max_distance {
                            max_distance = distance;
                            nearest_officer = Some(cs);
                        }
                    }
                    ProfileRank::Soldier
                        // Check if another soldier is already reporting to
                        // an officer — if so, don't duplicate the report.
                        if is_alerting_an_officer(cs.ai_substate)
                            && self.is_detecting_360_degrees(cs.handle, ctx)
                        => {
                            // Another soldier is already alerting an
                            // officer — abort.
                            return false;
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
        let officer_target_pos = officer
            .forecast_destination
            .as_ref()
            .unwrap_or_else(|| {
                panic!(
                    "AlertOfficer selected soldier {} without a required destination forecast",
                    officer.handle
                )
            })
            .resolve(sim)
            .position;

        self.current_task_priority = task_priority::ALERT;
        self.base.antagonist = Some(AiEntityHandle::new(officer_handle));
        self.gather_position = officer_target_pos;
        self.go_near(
            AiState::Seeking,
            Substate::SeekingRunningToOfficer,
            officer_target_pos,
            parameters_ai::AI_TALK_DISTANCE,
            GotoFlags::RUN,
            ctx,
        );
        // Track the officer so the soldier can detect them on the way.
        // Original does this after SetState and before GoNear. `go_near`
        // combines those operations, so the Rust statement has to follow the
        // wrapper call. This still preserves Original's effect order: the
        // engine drains actor-core effects (including this append) before it
        // launches and preflights the queued movement order. Consequently the
        // detectable is retained even when route construction fails.
        //
        // Original AddDetectable only asserts uniqueness in debug builds,
        // then appends unconditionally. The shipped release can therefore
        // retain a second Friend entry when CreateListOfSoldiersYouCanAlert
        // already registered this officer.
        self.base.outbox.actor.append_detectables.push((
            crate::element::EntityId::Soldier(crate::entity_id::SoldierId(officer_handle)),
            crate::element::DetectableType::Friend,
        ));
        self.base.launch_timer(50, ctx.frame);

        if self.base.couldnt_reachpoint {
            self.base.couldnt_reachpoint = false;
            return false;
        }

        true
    }

    /// Whether AlertOfficer will take its REPORT_OFFICER_AFTER instructed-
    /// group early return. That source branch returns immediately after
    /// GoNear and deliberately skips the ordinary final route-failure test.
    pub(crate) fn alert_officer_returns_to_instructed_group(&self, tick: &AiPerTickData) -> bool {
        self.seek_flags.contains(SeekFlags::REPORT_OFFICER_AFTER)
            && self.base.antagonist.is_some()
            && tick.camp_soldiers.iter().any(|soldier| {
                self.base
                    .antagonist
                    .is_some_and(|handle| handle.get() == soldier.handle)
                    && soldier.ai_substate == Substate::SeekingOfficerWaitForInstructedGroup
            })
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
            .outbox
            .actor
            .delete_detectables
            .push(crate::element::DetectableType::Friend);
        // Remember the alert point; clear body unless this is a
        // body-alert.
        self.base.alert_soldiers_point = position;
        if reason != ReportType::Body {
            self.base.detected_body = None;
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
        let patrol_head: Option<NpcHandle> = self.base.patrol.first().copied().and_then(|id| {
            tick.camp_soldiers.iter().find_map(|cs| {
                if cs.handle != id.index() {
                    return None;
                }
                let allowed = match cs.rank {
                    ProfileRank::Soldier => allow_soldier,
                    ProfileRank::Officer => allow_officer,
                    ProfileRank::Knight => allow_knight,
                    ProfileRank::None => false,
                };
                allowed.then_some(id.index())
            })
        });
        if let Some(h) = patrol_head {
            self.base.outbox.actor.add_detectables.push((
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
            self.base.outbox.actor.add_detectables.push((
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
        my_body_world: crate::coordinates::WorldPoint3D,
        max_radius: u16,
        rank: ProfileRank,
        tick: &AiPerTickData,
    ) -> Option<NpcHandle> {
        // `RHArtificialMalignity::GetNearestFighter`
        // (`RHartificialmalignity.cpp:16020-16137`) seeds its running minimum
        // with the squared radius and keeps the `<=` comparison, so the radius
        // itself is admissible and ties resolve to the *last* candidate in
        // fighter order.  Both the distance and the bound are `ULONG`s there —
        // the float square distance is truncated before either comparison.
        let max_sq: u32 = u32::from(max_radius) * u32::from(max_radius);
        let mut min_sq = max_sq;
        let mut best: Option<NpcHandle> = None;
        for cs in &tick.camp_soldiers {
            // GetNearestFighter's general gates are distinct from the global
            // camp registry used by CreateListOfSoldiersYouCanAlert: the
            // registry includes inactive and unconscious soldiers, while this
            // scan rejects inactive plus dead/unconscious candidates.
            // TODO(parity): the Original's gate is exactly
            // `!IsDead() && !IsUnconscious() && IsActive()`; `is_able_to_help`
            // additionally folds in `IsAbleToFight` and an AI-state whitelist.
            // Given the condition flag below already restricts the state, the
            // surviving difference is `IsAbleToFight`'s extra rejections
            // (tied/carried/hit-stun), which can only drop candidates.
            if !cs.active || !cs.is_able_to_help {
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

            // `RHArtificialIntelligence::SquareDistance`
            // (`RHartificialintelligence.cpp:6919-6922`) is
            // `( pSomething->GetPosition() - mpMe->GetPosition() )
            //   .StretchY( INVERSE_ASPECT_RATIO ).SquareNorm()` — a **3D**
            // squared norm (`SB3DStuff.h:64`) over the raw element points, so
            // the elevation term counts. Measuring it flat in map space made a
            // soldier two storeys up on a rampart look like a 172-unit
            // neighbour when the Original sees him more than 570 away.
            let dx = cs.position_world.x - my_body_world.x;
            let dy = (cs.position_world.y - my_body_world.y) * INVERSE_ASPECT_RATIO;
            let dz = cs.position_world.z - my_body_world.z;
            let sq = (dx * dx + dy * dy + dz * dz) as u32;
            if sq <= min_sq {
                min_sq = sq;
                best = Some(cs.handle);
            }
        }
        best
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
        let mut soldier: Option<(NpcHandle, Position)> =
            self.base.patrol.first().copied().and_then(|id| {
                tick.camp_soldiers.iter().find_map(|cs| {
                    (cs.handle == id.index() && cs.rank == ProfileRank::Soldier)
                        .then_some((cs.handle, cs.position))
                })
            });

        if soldier.is_none() {
            // GetNearestFighter(camp, 200,
            //   CONDITION_IS_IN_DEFAULT_STATE_OR_LOOKING_BODY,
            //   RANK_SOLDIER).
            if let Some(h) = self.get_nearest_fighter_default_or_looking_body(
                ctx.self_body_position_world,
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
    /// `CALL_TOWER_GUARD_ALERT` stimulus via the synchronous owner-boundary
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
        let mut simple_soldiers_who_hear_me = 0usize;
        let mut nearest_officer: Option<(crate::ai::NpcHandle, u32)> = None;
        let mut nearest_far_officer: Option<(crate::ai::NpcHandle, u32, WorldPoint3D)> = None;
        let sqr_radius = combat::SQR_TOWER_GUARD_ALERT_RADIUS as u32;
        let tower_position = WorldPoint3D {
            x: my_pos.x,
            y: my_pos.y + ctx.elevation,
            z: ctx.elevation,
        };

        // GetSoldier(camp, i) walks the camp registry in stable order.  Do
        // not iterate the entity-view HashMap here: delivery is synchronous,
        // so recipient order is observable through re-entrant AI callbacks.
        for soldier in &tick.camp_soldiers {
            let handle = soldier.handle;
            let view = ctx.entity_view(handle).unwrap_or_else(|| {
                panic!("tower-guard camp soldier {handle} is missing its entity view")
            });
            if !soldier.is_able_to_help {
                continue;
            }
            if soldier.is_tower_guard {
                continue;
            }
            if view.in_building {
                continue;
            }
            // SquareDistance reads GetPosition(), whose horizontal Y is map
            // Y plus ground elevation, before applying the aspect stretch.
            // Comparing projected map Y wrongly excludes soldiers on a
            // different elevation from the guard.  The norm it takes is the
            // full three-dimensional one, so the height gap between a tower
            // guard and the ground below counts toward the alert radius: drop
            // the Z term and the cry reaches soldiers standing too far below.
            let sq_dist = tower_guard_square_distance(soldier.position_world, tower_position);

            if sq_dist < sqr_radius {
                // This soldier hears the cry. Rank
                // classification happens in this same camp-registry walk in
                // the reference, reusing the exact SquareDistance result.
                in_range_soldiers.push(handle);
                match soldier.rank {
                    ProfileRank::Soldier if nearest_officer.is_none() => {
                        // Original stores the soldier in
                        // listSimpleSoldiersWhoHearMe here. Its later runner
                        // scan accidentally uses only the list's size, not
                        // its contents; retain that observable count.
                        simple_soldiers_who_hear_me += 1;
                    }
                    ProfileRank::Officer
                        if nearest_officer.is_none_or(|(_, distance)| sq_dist < distance) =>
                    {
                        nearest_officer = Some((handle, sq_dist));
                    }
                    _ => {}
                }
            } else if soldier.rank == ProfileRank::Officer
                && sq_dist < nearest_far_officer.map(|(_, d, _)| d).unwrap_or(u32::MAX)
            {
                // Only consider RANK_OFFICER for the
                // far-officer fallback.  AiEntityView carries `rank`
                // so we can apply the same gate here.
                nearest_far_officer = Some((handle, sq_dist, soldier.position_world));
            }
        }

        // Queue the in-range alerts. Dispatch these
        // synchronously in-loop, but the Rust engine's deferred
        // cross-NPC action pass delivers them later in the same
        // tick — same observable ordering for the target.
        for handle in &in_range_soldiers {
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::SendStimulus {
                    target: *handle,
                    stimulus_type: StimulusType::CallTowerGuardAlert,
                    info: StimulusInfo::Hint(alert_hint),
                    fallback_to_sender: None,
                    to_whole_patrol: false,
                });
        }

        if let Some((officer, _)) = nearest_officer {
            // Directly alert the officer — they'll come
            // investigate via `CallTowerGuardCallsMeStandardProcedure`.
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::SendStimulus {
                    target: officer,
                    stimulus_type: StimulusType::CallTowerGuardCallsMe,
                    info: StimulusInfo::Hint(alert_hint),
                    fallback_to_sender: None,
                    to_whole_patrol: false,
                });
            return;
        }

        // No in-range officer — look for a runner. Preserve two observable
        // Original bugs: the loop is bounded by the hearing-list size but
        // dereferences GetSoldier(camp, i), and each candidate is compared
        // with the far officer's tower distance instead of the previously
        // selected runner distance. The result is the last qualifying actor
        // in the first N camp-registry entries, not the nearest hearer.
        let Some((_, officer_distance, officer_pos)) = nearest_far_officer else {
            return;
        };
        let complete_registry = tower_guard_complete_registry(
            tick.camp_soldiers
                .iter()
                .map(|soldier| (soldier.handle, soldier.position_world)),
            (self.base.me, tower_position),
        );
        let runner = tower_guard_runner_from_registry_prefix(
            complete_registry,
            simple_soldiers_who_hear_me,
            officer_pos,
            officer_distance,
        );

        if let Some(runner_handle) = runner {
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
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
        self.base.outbox.actor.set_unfocus();

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
        self.base.my_door_index = Some(door.door_index);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn alert_candidate(handle: u32, position: Position) -> CampSoldierInfo {
        CampSoldierInfo {
            handle,
            active: true,
            position,
            position_world: WorldPoint3D::new(position.x, position.y, 0.0),
            direction: 0,
            rank: ProfileRank::Soldier,
            ai_state: AiState::Default,
            ai_substate: Substate::DefaultOnPost,
            is_able_to_fight: true,
            is_dead: false,
            knocked_out_in_money_fight: false,
            primary_target: 0,
            pride: 0,
            is_able_to_help: true,
            script_locked: false,
            ai_lock_frozen: false,
            layer: 0,
            report_type: ReportType::Nothing,
            report_seek_position: Position::default(),
            report_seen_bodies: Vec::new(),
            report_charly: 0,
            alert_soldiers_point: Position::default(),
            patrol_chief: None,
            antagonist: 0,
            detected_body: 0,
            blood_alcohol: 0,
            duty_flag: false,
            is_tower_guard: false,
            company_number: 0,
            in_building: false,
            forecast_destination: None,
            detectable_bodies: Vec::new(),
            seek_position: Position::default(),
            current_task_priority: 0,
            minimal_task_priority: 0,
            view_direction: [1.0, 0.0],
            view_radius: 300,
            real_half_aperture: crate::ai_vision::NORMAL_HALF_APERTURE,
            eye_blind: false,
        }
    }

    fn soldier_entity_view(position: Position) -> crate::ai_entity_view::AiEntityView {
        let entity = crate::element::Entity::Soldier(crate::element::ActorSoldier {
            element: crate::element::ElementData::default(),
            actor: crate::element::ActorData::default(),
            human: crate::element::HumanData::default(),
            npc: crate::element::NpcData::default(),
            soldier: crate::element::SoldierData::default(),
        });
        let mut view = crate::ai_entity_view::entity_view_from_entity(
            &entity,
            127,
            false,
            None,
            None,
            crate::order::OrderType::WaitingUpright,
        );
        view.position = position;
        view
    }

    #[test]
    fn alert_soldier_radius_keeps_positive_strict_float_gates() {
        let officer = Position::default();
        let at = |x, y| Position {
            x,
            y,
            ..Position::default()
        };

        assert!(alert_soldier_is_inside_radius(at(3.0, 4.0), officer, 6.0));
        assert!(
            !alert_soldier_is_inside_radius(at(5.0, 0.0), officer, 5.0),
            "Original's strict MaxNorm comparison rejects the boundary"
        );
        assert!(
            !alert_soldier_is_inside_radius(at(3.0, 4.0), officer, 5.0),
            "Original's strict SquareNorm comparison rejects its boundary"
        );
        assert!(
            !alert_soldier_is_inside_radius(at(4.0, 4.0), officer, 5.0),
            "the squared-radius gate still rejects points inside the MaxNorm box"
        );
        assert!(
            !alert_soldier_is_inside_radius(at(f32::NAN, 0.0), officer, 5.0),
            "an unordered X coordinate must fail Original's positive comparisons"
        );
        assert!(
            !alert_soldier_is_inside_radius(at(0.0, f32::NAN), officer, 5.0),
            "an unordered Y coordinate must fail Original's positive comparisons"
        );
    }

    #[test]
    fn alert_soldiers_radius_uses_door_resolved_ai_position() {
        // Save018/r040: Soldier96's interpolated body lies inside the
        // officer's circle, while Position(Soldier96) resolves its active
        // door pass to gate 27's point-out and lies outside it. Original
        // therefore rejects the soldier before delivering CALL_ALERT.
        let officer = Position {
            x: 1723.7462,
            y: 747.5348,
            ..Position::default()
        };
        let raw_body = Position {
            x: 1458.0,
            y: 331.0,
            ..Position::default()
        };
        let gate_point_out = Position {
            x: 1416.0,
            y: 344.0,
            ..Position::default()
        };
        let radius = combat::ALERT_RADIUS as f32;
        assert!(alert_soldier_is_inside_radius(raw_body, officer, radius));
        assert!(!alert_soldier_is_inside_radius(
            gate_point_out,
            officer,
            radius
        ));

        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(96, soldier_entity_view(gate_point_out));
        let ctx = AiContext {
            position: officer,
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.camp_soldiers.push(alert_candidate(96, raw_body));

        let mut ai = EnemyAi::new(99);
        ai.soldier_profile_rank = ProfileRank::Officer;
        assert!(!ai.alert_soldiers(
            Position::default(),
            0,
            &AiGlobalState::default(),
            None,
            &ctx,
            &tick,
            AlertSoldiersFailureContinuation::None,
        ));
        assert!(ai.base.outbox.reentrant.cross_npc_actions.is_empty());
    }

    #[test]
    fn formation_direction_uses_original_aspect_ratio_classifier() {
        assert_eq!(formation_direction(1.0, 1.0), 7);
        assert_eq!(formation_direction(-1.0, 1.0), 9);
        assert_ne!(
            formation_direction(1.0, 1.0),
            crate::position_interface::vector_to_sector_0_to_15_with_aspect(1.0, 1.0, 1.0) as u16,
            "the aspect-1 classifier would start the formation sweep one sector early"
        );
    }

    #[test]
    fn formation_sweep_preserves_raw_cursor_after_sector_wrap() {
        // nicouzouf Save014/r013: the sweep starts at 8 and accepts its
        // fourteenth attempt, projected sector 5. Original retains raw cursor
        // 21 and instructs the soldiers with 21 ^ 8 = 29, not normalized 13.
        let (raw, projected) = formation_sweep_cursor(8, 13);
        assert_eq!(raw, 21);
        assert_eq!(projected, 5);
        assert_eq!(raw ^ 8, 29);
        assert_eq!((raw ^ 8) & 15, 13);
    }

    #[test]
    fn coincident_alerted_soldier_poisons_attack_formation_direction() {
        let officer = Position {
            x: 744.0,
            y: 675.0,
            ..Position::default()
        };
        let alerted = [
            Position {
                x: 683.0,
                y: 713.0,
                ..Position::default()
            },
            officer,
            Position {
                x: 486.0,
                y: 880.0,
                ..Position::default()
            },
        ];

        let average = average_alerted_direction_vector(officer, &alerted);
        assert!(average.0.is_nan() && average.1.is_nan());
        assert_eq!(formation_direction(average.0, average.1), 0);
    }

    #[test]
    fn inactive_duty_soldier_uses_indoor_stay_on_post_answer() {
        assert!(!alert_soldier_stays_on_post(
            false, false, false, true, 0, 0
        ));
        assert!(alert_soldier_stays_on_post(true, false, false, true, 0, 0));
        assert!(!alert_soldier_stays_on_post(true, true, false, true, 0, 0));
    }

    #[test]
    fn drunkenness_precedes_active_outdoor_stay_on_post_branch() {
        let limit = crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT as u8;
        assert!(!alert_soldier_stays_on_post(
            false, false, false, false, 0, limit
        ));
        assert!(alert_soldier_stays_on_post(
            false,
            false,
            false,
            false,
            0,
            limit + 1
        ));
        assert!(alert_soldier_stays_on_post(
            true,
            true,
            false,
            false,
            0,
            limit + 1
        ));
    }

    #[test]
    fn patrol_chief_can_alert_drunk_soldier_despite_stay_on_post_answer() {
        let candidate_position = Position {
            x: 10.0,
            ..Position::default()
        };
        let mut views = crate::ai_entity_view::AiEntityViewMap::new();
        views.insert(96, soldier_entity_view(candidate_position));
        let ctx = AiContext {
            entity_views: crate::ai_entity_view::shared_entity_views(views),
            ..AiContext::default()
        };

        let mut candidate = alert_candidate(96, candidate_position);
        candidate.blood_alcohol = (crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT + 1) as u8;
        let mut tick = AiPerTickData::stub();
        tick.camp_soldiers.push(candidate.clone());

        let mut unrelated_officer = EnemyAi::new(99);
        unrelated_officer.soldier_profile_rank = ProfileRank::Officer;
        assert!(!unrelated_officer.alert_soldiers(
            Position::default(),
            0,
            &AiGlobalState::default(),
            None,
            &ctx,
            &tick,
            AlertSoldiersFailureContinuation::None,
        ));

        tick.camp_soldiers[0].patrol_chief = Some(crate::element::EntityId::Soldier(
            crate::entity_id::SoldierId(99),
        ));
        let mut patrol_chief = EnemyAi::new(99);
        patrol_chief.soldier_profile_rank = ProfileRank::Officer;
        assert!(patrol_chief.alert_soldiers(
            Position::default(),
            0,
            &AiGlobalState::default(),
            None,
            &ctx,
            &tick,
            AlertSoldiersFailureContinuation::None,
        ));
        assert_eq!(
            patrol_chief.base.outbox.reentrant.cross_npc_actions.len(),
            1
        );
    }

    #[test]
    fn attack_point_direction_uses_world_y() {
        let ctx = AiContext {
            position: Position {
                x: 0.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            elevation: 100.0,
            ..AiContext::default()
        };
        let target = Position {
            x: 100.0,
            y: 100.0,
            sector: None,
            level: 0,
        };

        // Both ground points have world Y = 100, so the target is due east.
        // Classifying their projected-map delta would incorrectly include a
        // +100 Y component.
        assert_eq!(
            attack_point_direction(&ctx, target),
            vec_to_sector(100.0, 0.0)
        );
        assert_ne!(
            attack_point_direction(&ctx, target),
            vec_to_sector(100.0, 100.0)
        );
    }

    #[test]
    fn alert_officer_distance_uses_world_y_before_isometric_stretch() {
        let owner = Position {
            x: 1173.4828,
            y: 1187.3944,
            sector: None,
            level: 2,
        };
        let officer_47 = Position {
            x: 802.99005,
            y: 1669.0012,
            sector: None,
            level: 0,
        };
        let officer_66 = Position {
            x: 363.0,
            y: 1118.0,
            sector: None,
            level: 0,
        };

        let distance_47 = alert_officer_distance(&officer_47, 0.0, 0, false, &owner, 110.001, 2);
        let distance_66 = alert_officer_distance(&officer_66, 0.0, 0, false, &owner, 110.001, 2);

        assert!(distance_47 < distance_66);
        // Raw map-Y incorrectly reverses this ordering for the Derby control.
        let raw_map_distance_47 = (officer_47.x - owner.x)
            .abs()
            .max((officer_47.y - owner.y).abs() * INVERSE_ASPECT_RATIO);
        let raw_map_distance_66 = (officer_66.x - owner.x)
            .abs()
            .max((officer_66.y - owner.y).abs() * INVERSE_ASPECT_RATIO);
        assert!(raw_map_distance_47 > raw_map_distance_66);
    }

    #[test]
    fn alert_officer_rejects_inactive_default_officer_able_to_help() {
        // nicouzouf Savegame_067 replay-008, frame 1509: Officer55 is
        // inactive inside a building. Original IsAbleToFight rejects it,
        // while IsAbleToHelp would accept its Default state.
        let is_able_to_help =
            crate::ai_enemy::soldier_is_able_to_help_state(true, AiState::Default, Substate::None);
        assert!(is_able_to_help, "the mismatched legacy predicate admits it");
        assert!(!can_alert_officer(
            ProfileRank::Officer,
            false,
            AiState::Default,
            false,
        ));
        assert!(can_alert_officer(
            ProfileRank::Officer,
            true,
            AiState::Default,
            false,
        ));
    }

    #[test]
    fn alert_officer_scan_includes_reporting_owner_substates() {
        // Nescafe Savegame_001 replay-007, frame 1539: Soldier139 reaches a
        // now-busy officer while already in RUNNING_TO_OFFICER. Original's
        // global camp scan encounters Soldier139 itself and refuses to pick
        // another officer; the owner-omitting Rust snapshot used to select
        // Officer124 and launch an extra movement path instead.
        for substate in [
            Substate::SeekingSoldierCalledByOfficer,
            Substate::SeekingSoldierGoToOfficer,
            Substate::SeekingSoldierGetInstructedByOfficer,
            Substate::SeekingSoldierReturnToOfficer,
            Substate::SeekingSoldierGiveReportToOfficer,
            Substate::SeekingSoldierGiveAlertingReportToOfficerStart,
            Substate::SeekingSoldierGiveAlertingReportToOfficerPoint,
            Substate::SeekingSoldierGiveAlertingReportToOfficerEnd,
            Substate::SeekingGroupCalledByOfficer,
            Substate::SeekingGroupGoToOfficer,
            Substate::SeekingGroupGetInstructedByOfficer,
            Substate::SeekingRunningToOfficer,
            Substate::SeekingRunningToOfficerSeen,
        ] {
            assert!(is_alerting_an_officer(substate), "{substate:?}");
        }
        assert!(!is_alerting_an_officer(Substate::DefaultOnPost));
        assert!(!is_alerting_an_officer(
            Substate::SeekingOfficerWaitForInstructedSoldier
        ));
    }

    #[test]
    fn alert_officer_splices_owner_into_registry_order() {
        // Linux3/Profile003/Save019/r003 frame 18995: reporting Soldier100
        // precedes the evaluating Soldier101. Original observes Soldier100's
        // visibility query and aborts before it ever reaches the owner.
        let without_owner = [100_u32, 102];
        assert_eq!(
            sorted_owner_insertion_index(&without_owner, 101, |handle| *handle),
            1
        );
        assert_eq!(
            sorted_owner_insertion_index(&without_owner, 99, |handle| *handle),
            0
        );
        assert_eq!(
            sorted_owner_insertion_index(&without_owner, 103, |handle| *handle),
            2
        );
    }

    #[test]
    fn alert_soldier_sort_distance_uses_literal_3d_position() {
        let officer = WorldPoint3D::new(884.283, 565.7891, 0.0);
        let soldier_53 = WorldPoint3D::new(1056.0076, 220.9945, 36.001);
        let soldier_54 = WorldPoint3D::new(1191.9963, 250.0275, 0.0);

        let distance_53 = alert_soldier_sort_distance(soldier_53, officer);
        let distance_54 = alert_soldier_sort_distance(soldier_54, officer);
        assert!(
            distance_54 > distance_53,
            "Original's farthest-first list puts ground-level Soldier54 before elevated Soldier53"
        );

        let officer_map = officer.to_map();
        let soldier_53_map = soldier_53.to_map();
        let soldier_54_map = soldier_54.to_map();
        let raw_map_distance_53 = (soldier_53_map.x - officer_map.x).powi(2)
            + ((soldier_53_map.y - officer_map.y) * INVERSE_ASPECT_RATIO).powi(2);
        let raw_map_distance_54 = (soldier_54_map.x - officer_map.x).powi(2)
            + ((soldier_54_map.y - officer_map.y) * INVERSE_ASPECT_RATIO).powi(2);
        assert!(
            raw_map_distance_53 > raw_map_distance_54,
            "the old projected-2D shortcut must demonstrate the representative reversal"
        );
    }

    #[test]
    fn alert_soldier_sort_is_farthest_first_and_later_first_on_ties() {
        let mut alerted = [(51, 25.0, 0), (52, 100.0, 1), (53, 25.0, 2)];

        sort_alerted_soldiers(&mut alerted);

        assert_eq!(
            alerted.map(|(handle, _, _)| handle),
            [52, 53, 51],
            "Original inserts farther soldiers first and equal-distance newcomers before older entries"
        );
    }

    #[test]
    fn tower_guard_runner_uses_registry_prefix_and_last_qualifier() {
        let position = |x| WorldPoint3D { x, y: 0.0, z: 0.0 };
        let registry = [
            (10, position(50.0)),
            (11, position(5.0)),
            (12, position(9.0)),
            (13, position(1.0)),
        ];

        assert_eq!(
            tower_guard_runner_from_registry_prefix(registry, 3, position(0.0), 100),
            Some(12),
            "Original ignores the closest actor outside the first N registry entries and keeps the last qualifying prefix actor"
        );
    }

    #[test]
    fn tower_guard_runner_prefix_restores_owner_at_registry_position() {
        let position = |x| WorldPoint3D { x, y: 0.0, z: 0.0 };
        let registry_without_owner = [
            (105, position(20.0)),
            (108, position(5.0)),
            (109, position(4.0)),
        ];
        let complete = tower_guard_complete_registry(registry_without_owner, (106, position(10.0)));

        assert_eq!(
            tower_guard_runner_from_registry_prefix(complete, 2, position(0.0), 100),
            None,
            "the tower guard occupies its Original registry slot, so the later Rust-only runner must remain outside the first-N prefix"
        );
    }

    #[test]
    fn tower_guard_runner_truncates_squared_distances_before_comparing() {
        let position = |x| WorldPoint3D { x, y: 0.0, z: 0.0 };
        let officer_distance = tower_guard_square_distance(position(0.0), position(10.04));

        assert_eq!(officer_distance, 100);
        assert_eq!(
            tower_guard_runner_from_registry_prefix(
                [(7, position(10.02))],
                1,
                position(0.0),
                officer_distance,
            ),
            None,
            "100.4004 and 100.8016 both truncate to ULONG 100, so strict less-than must reject the candidate"
        );
    }
}
