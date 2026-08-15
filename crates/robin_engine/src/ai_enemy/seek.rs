//! Seek-and-search behaviours: the area-search seek-point loop, body
//! examination, search-for-charly, run-to-free-net-victim,
//! find-door-enemy-could-be-behind, dead-body-alert dispatch, and the
//! flee primitive.

use crate::ai::*;
use crate::parameters_ai;
use crate::position_interface::INVERSE_ASPECT_RATIO;

fn seek_area_selection_debug_matches(frame: u32, creation_order: Option<u32>) -> bool {
    if std::env::var_os("PARITY_DEBUG_SEEK_AREA_OWNER_POSITION").is_none() {
        return false;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for SEEKAREA diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_SEEK_AREA_FRAME").is_none_or(|expected| frame == expected)
        && parse_filter("PARITY_DEBUG_SEEK_AREA_CREATION_ORDER")
            .is_none_or(|expected| creation_order == Some(expected))
}

fn seek_area_phase6_debug_enabled() -> bool {
    std::env::var_os("PARITY_DEBUG_SEEK_AREA_PHASE6").is_some()
}

fn seek_area_phase6_debug_matches(frame: u32, creation_order: Option<u32>) -> bool {
    if !seek_area_phase6_debug_enabled() {
        return false;
    }
    let parse_required = |name: &str| {
        let value = std::env::var(name)
            .unwrap_or_else(|_| panic!("PARITY_DEBUG_SEEK_AREA_PHASE6 requires {name}"));
        if value.is_empty() {
            panic!("PARITY_DEBUG_SEEK_AREA_PHASE6 requires non-empty {name}");
        }
        value.parse::<u32>().unwrap_or_else(|error| {
            panic!("invalid {name}={value:?} for SEEKAREA phase6 diagnostic: {error}")
        })
    };
    let expected_frame = parse_required("PARITY_DEBUG_SEEK_AREA_FRAME");
    let expected_owner = parse_required("PARITY_DEBUG_SEEK_AREA_CREATION_ORDER");
    frame == expected_frame && creation_order == Some(expected_owner)
}

#[inline]
fn accumulate_seek_point_interest(current: f32, interest: u8) -> f32 {
    (f64::from(current) + f64::from(interest) * 0.01_f64) as f32
}

use super::util::{pos_distance, resolve_seek_point_id, resolve_seek_point_mut, vec_to_sector};
use super::{EnemyAi, ProfileRank, SeekFlags, UNDEFINED_DIRECTION, task_priority};

impl EnemyAi {
    pub(crate) fn seek_area_phase6_caller_debug_enabled() -> bool {
        seek_area_phase6_debug_enabled()
    }

    pub(crate) fn seek_area_phase6_caller_debug_matches(
        frame: u32,
        creation_order: Option<u32>,
    ) -> bool {
        seek_area_phase6_debug_matches(frame, creation_order)
    }

    // -----------------------------------------------------------------------
    // Flee
    // -----------------------------------------------------------------------

    pub fn flee(
        &mut self,
        danger_pos: &Position,
        ctx: &AiContext,
        _tick: &AiPerTickData,
        global: &AiGlobalState,
    ) {
        self.base.say(Remark::Panic);

        // Flee AWAY from danger. Iterate global seek points and find
        // the farthest safe point in the flee direction (dot product
        // > 0 means same direction as danger→me vector).
        let danger_to_me = (ctx.position.x - danger_pos.x, ctx.position.y - danger_pos.y);

        let mut best_point: Option<Position> = None;
        let mut max_distance: f32 = 100.0;

        for sp in &global.seek_points {
            let danger_to_sp = (sp.position.x - danger_pos.x, sp.position.y - danger_pos.y);
            // Dot product: positive means the seek point is in the
            // flee direction (away from danger).
            let dot = danger_to_sp.0 * danger_to_me.0 + danger_to_sp.1 * danger_to_me.1;
            if dot > 0.0 {
                let dist = danger_to_sp.0.abs().max(danger_to_sp.1.abs()); // MaxNorm
                if dist > max_distance {
                    max_distance = dist;
                    best_point = Some(sp.position);
                }
            }
        }

        let Some(flee_pos) = best_point else {
            // The reference asserts here, which in release builds is a
            // no-op and the function returns without a state change.
            // Per CLAUDE.md "no fake data", we log a warning and
            // early-return rather than fabricating a 500-unit synthetic
            // flee destination.
            tracing::warn!(
                me = self.base.me,
                "flee: no seek point with positive danger-flee dot product"
            );
            return;
        };

        // Store the DANGER position in seek_position (not the flee
        // destination). Used by the Cassos decision to re-flee.
        self.base.seek_position = *danger_pos;
        self.base.set_emoticon(EmoticonType::XMark);
        self.go_to(
            AiState::Fleeing,
            Substate::FleeingRunToHide,
            flee_pos,
            crate::ai::GotoFlags::RUN,
            ctx,
        );
    }

    // -----------------------------------------------------------------------
    // SeekArea — seek the environment after losing sight of enemy
    // -----------------------------------------------------------------------

    /// Begin a search pattern around `center`. Selects seek points from
    /// the global array based on distance, interest, and direction, then
    /// visits them in an optimised order.
    #[allow(clippy::too_many_arguments)]
    pub fn seek_area(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        center: Position,
        standard_radius: u16,
        flags: SeekFlags,
        seek_direction: u16,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        tracing::trace!(
            npc = self.base.me,
            state = ?self.base.current_state,
            substate = ?self.base.current_substate,
            center_x = center.x,
            center_y = center.y,
            standard_radius,
            ?flags,
            seek_direction,
            "entering SeekArea"
        );
        self.base.stop_all();

        // Focus(NULL): clear any prior stare-at-target focus so the
        // eye-tracking view cone doesn't stick on a stale primary
        // target while we sweep seek points. Drained by `engine/ai.rs`
        // → `unfocus`.
        self.base.outbox.actor.set_unfocus();

        // Royalists just return to duty.
        if ctx.camp == crate::element::Camp::Royalists {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            return;
        }

        // Company 100 (combat trainer dummy) just returns to duty.
        if self.company_number == 100 {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            return;
        }

        if !flags.contains(SeekFlags::CHARLY_SEEK) {
            // SetCheckpointCharly(NULL) — route through the helper so
            // the `DETECTABLE_MISSED_FRIEND` list is cleared and
            // `sorrow_level` is zeroed alongside the field write.
            self.base.set_checkpoint_charly(0);
        }

        self.current_task_priority = task_priority::SEEKING;

        // Before launching the seek-area proper, check whether any
        // previously-seen body still needs investigating. If so, defer
        // the seek entirely and let `run_to_examine_body` drive the NPC
        // to the body. `examine_other_bodies` prunes recovered bodies
        // from the queue automatically.
        if self.examine_other_bodies(ctx, tick) {
            return;
        }

        // For IQ ≥ `CHECK_BEGGAR_MIN_IQ` non-trainer soldiers, Original
        // clears `DETECTABLE_BEGGAR` and immediately re-adds every actor for
        // which `IsTrueOrFalseBeggar()` holds. This is authoritative list
        // state, not merely preparation for the next detection refresh: a
        // frame dump taken after SeekArea already contains the rebuilt list.
        if (self.get_iq(ctx) as i32) >= parameters_ai::CHECK_BEGGAR_MIN_IQ && !self.combat_trainer {
            use crate::element::{DetectableType, Posture};

            self.base
                .outbox
                .actor
                .add_detectables
                .retain(|(_, detectable_type)| *detectable_type != DetectableType::Beggar);
            self.base
                .outbox
                .actor
                .delete_detectables
                .push(DetectableType::Beggar);
            let mut beggars: Vec<_> = ctx
                .entity_views
                .iter()
                .filter_map(|(&handle, view)| {
                    let is_true_or_false_beggar = (view.is_civilian() && view.is_beggar)
                        || ((view.is_pc || view.is_soldier())
                            && view.posture == Posture::SimulatingBeggar);
                    is_true_or_false_beggar.then(|| {
                        (
                            view.original_creation_order,
                            handle,
                            view.entity_id(handle).unwrap_or_else(|| {
                                panic!("beggar actor {handle} has no typed entity identity")
                            }),
                        )
                    })
                })
                .collect();
            beggars.sort_unstable_by_key(|&(creation_order, handle, _)| (creation_order, handle));
            self.base.outbox.actor.add_detectables.extend(
                beggars
                    .into_iter()
                    .map(|(_, _, entity_id)| (entity_id, DetectableType::Beggar)),
            );
            self.beggar_to_examine = 0;
        }

        // Store seek flags and center
        self.seek_flags =
            flags | (flags & (SeekFlags::LOOK_FOR_HELP_AFTER | SeekFlags::REPORT_OFFICER_AFTER));
        self.seek_center = center;
        self.my_seek_points.clear();
        self.seek_point_view_directions.clear();

        let current_frame = ctx.frame;

        // ── Build seek point list from global array ──
        // Gate on `standard_radius > 0 && !is_combat_trainer`. Combat
        // trainers fall through to the `LOCATION_FIRST/END`
        // assert/personal-seek-point branch.
        if standard_radius > 0 && !self.combat_trainer {
            let sq_standard_radius = (standard_radius as f32) * (standard_radius as f32);
            let mut obligatory_idx: Option<usize> = None;
            let mut obligatory2_idx: Option<usize> = None;
            // Original seeds both FLOAT minima with its local `oo` macro
            // (65432), not floating-point infinity. Direction candidates
            // beyond that squared distance cannot become obligatory.
            let mut min_sqr_norm: f32 = 65_432.0;
            let mut min_sqr_norm2: f32 = 65_432.0;
            let mut expected_points_for_one = 1u16;
            let mut square_norms = vec![f32::MAX; global.seek_points.len()];

            // ── Phase 1: compute distances, find obligatory point ──
            for (i, sp) in global.seek_points.iter().enumerate() {
                let dx = sp.position.x - center.x;
                let dy = sp.position.y - center.y;
                let mut square_norm = dx * dx + dy * dy;

                // Penalty for layer changes
                if sp.position.level != center.level {
                    square_norm += parameters_ai::LAYER_CHANGE_PENALTY
                        * (sp.position.level as f32 - center.level as f32).abs();
                }
                square_norms[i] = square_norm;

                // Count points in radius (for expected count)
                if square_norm < sq_standard_radius {
                    expected_points_for_one += 1;
                }

                // Check if this point is in the seek direction.
                // The reference uses `% 15` (not `& 15`/`% 16`),
                // making case 15 unreachable — port the bug literally
                // so sector-bucket assignments match for boundary
                // sectors (e.g. seek_direction=0, sector 14:
                // (14+16)%15 = 0 → "in direction"; & 15 would give 14
                // → "almost").
                if seek_direction != UNDEFINED_DIRECTION {
                    let dir_sector = vec_to_sector(dx, dy);
                    let diff = (dir_sector + 16 - seek_direction) % 15;
                    match diff {
                        15 | 0 | 1
                            if square_norm < min_sqr_norm && sp.position.level == center.level =>
                        {
                            obligatory_idx = Some(i);
                            min_sqr_norm = square_norm;
                        }
                        14 | 2
                            if square_norm < min_sqr_norm2 && sp.position.level == center.level =>
                        {
                            obligatory2_idx = Some(i);
                            min_sqr_norm2 = square_norm;
                        }
                        _ => {}
                    }
                }
            }

            // Fallback obligatory
            if obligatory_idx.is_none() {
                obligatory_idx = obligatory2_idx;
            }

            // ── Phase 2: collect seek points within max radius, sorted by distance ──
            let mut near_sorted: Vec<usize> = Vec::new();
            for (i, &square_norm) in square_norms.iter().enumerate() {
                if square_norm < parameters_ai::SEEK_POINT_MAX_SQR_RADIUS as f32 {
                    // Insert sorted by distance
                    let pos = near_sorted
                        .iter()
                        .position(|&idx| square_norms[idx] > square_norm)
                        .unwrap_or(near_sorted.len());
                    near_sorted.insert(pos, i);
                }
            }

            if seek_area_selection_debug_matches(ctx.frame, ctx.original_creation_order) {
                for (i, sp) in global.seek_points.iter().enumerate() {
                    eprintln!(
                        "SEEKAREA {{\"event\":\"point_dump\",\"frame\":{},\"index\":{},\"id\":{},\"x\":{},\"y\":{},\"level\":{},\"center\":[{},{},{}],\"norm\":{},\"norm_bits\":{},\"near\":{}}}",
                        ctx.frame,
                        i,
                        sp.id,
                        sp.position.x,
                        sp.position.y,
                        sp.position.level,
                        center.x,
                        center.y,
                        center.level,
                        square_norms[i],
                        square_norms[i].to_bits(),
                        near_sorted.contains(&i),
                    );
                }
            }

            // If nearest point was recently examined, don't look for help
            if let Some(&first_idx) = near_sorted.first()
                && global.seek_points[first_idx].calculate_interest(current_frame) < 90
            {
                self.seek_flags &= !SeekFlags::LOOK_FOR_HELP_AFTER;
            }

            // ── Phase 3: friend coordination ──
            // Walk every NPC and count visible friend soldiers within
            // 500 units in alert > Green. Each friend multiplies the
            // expected point count by `SEEK_POINT_NUMBER_FACTOR`. The
            // engine pre-fills the count and the help-flag clear bit
            // before think().
            //
            // The lock on each seek point provides real-time
            // coordination (a soldier won't pick a point another
            // soldier is already running to); the friend count
            // determines how many points each soldier signs up for.
            let mut friend_factor: f32 = 1.0;
            for _ in 0..tick.visible_seeking_friends {
                friend_factor *= parameters_ai::SEEK_POINT_NUMBER_FACTOR;
            }
            if tick.friend_seek_clears_help_flag {
                self.seek_flags &= !SeekFlags::LOOK_FOR_HELP_AFTER;
            }

            let mut expected_points = (expected_points_for_one as f32 * friend_factor) as u16;
            let expected_points_before_help_random = expected_points;
            let mut preselection_rng_draws = 0usize;

            if self.seek_flags.contains(SeekFlags::LOOK_FOR_HELP_AFTER) {
                // Reduce seek count when planning to ask for help.
                // The reference's `Consider(COURAGE)` call has an
                // entirely commented-out switch body — it sets
                // `bPositively` but never accumulates anything onto
                // `sum_of_values_to_consider` / `sum_of_weights`.
                // Combined with `P_RECTANGLE` being a plain `min + rand()
                // % range` that never reads `EvaluateConsiderations()`,
                // the courage bias is a no-op. Rust's uniform sample
                // matches. The courage axis itself *is* ported
                // (`AiBrain::soldier_profile_courage` / `get_courage`),
                // wired into the call sites that actually use it
                // (`CHARGE_MIN_COURAGE`, `OBSERVE_SWORDFIGHT` distance,
                // courage_distance, etc).
                let min = (expected_points as f32
                    * parameters_ai::AI_MIN_LOOKFORHELPFLAG_SEEK_POINT_FACTOR)
                    as u16;

                // Original `RandomValue(P_RECTANGLE, min, max)` returns
                // `min + rand() % (max - min)`: the upper bound is excluded,
                // and an empty span returns `min` without consuming RNG.
                expected_points = if min == expected_points {
                    min
                } else {
                    preselection_rng_draws += 1;
                    crate::sim_rng::u16(
                        sim,
                        crate::sim_rng::RngSite::SeekPointSelection,
                        min..expected_points,
                    )
                };
            }

            // ── Phase 4: select points by interest (randomised order) ──
            let mut selected_random: Vec<usize> = Vec::new();
            let mut count_f: f32 = 0.0;
            let mut phase4_attempts = 0usize;
            let mut phase4_accepts = 0usize;
            let debug_selection =
                seek_area_selection_debug_matches(ctx.frame, ctx.original_creation_order);

            for &idx in &near_sorted {
                if count_f >= expected_points as f32 {
                    break;
                }
                let accumulator_before_bits = count_f.to_bits();
                let interest = global.seek_points[idx].calculate_interest(current_frame);
                phase4_attempts += 1;
                let attempt =
                    crate::sim_rng::u8(sim, crate::sim_rng::RngSite::SeekPointSelection, 0..100);
                let attempt_raw = debug_selection
                    .then(|| crate::sim_rng::last_original_raw_draw(sim))
                    .flatten();
                let accepted = attempt < interest;
                let mut insertion_raw = None;
                let mut insertion_index = None;
                if accepted {
                    phase4_accepts += 1;
                    // Unconditionally call rand on every accepted
                    // point, including the first (where the count == 1
                    // consumes a draw deterministically returning 0).
                    // Match the RNG-step count exactly for replay
                    // determinism — no `is_empty()` short-circuit.
                    let insert_pos = crate::sim_rng::usize(
                        sim,
                        crate::sim_rng::RngSite::SeekPointSelection,
                        0..=selected_random.len(),
                    );
                    insertion_raw = debug_selection
                        .then(|| crate::sim_rng::last_original_raw_draw(sim))
                        .flatten();
                    insertion_index = Some(insert_pos);
                    selected_random.insert(insert_pos, idx);
                    count_f = accumulate_seek_point_interest(count_f, interest);
                }

                if debug_selection {
                    let optional_u32 = |value: Option<u32>| {
                        value.map_or_else(|| "null".to_owned(), |value| value.to_string())
                    };
                    let optional_usize = |value: Option<usize>| {
                        value.map_or_else(|| "null".to_owned(), |value| value.to_string())
                    };
                    eprintln!(
                        "SEEKAREA {{\"event\":\"phase4_candidate\",\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{},\"candidate_ordinal\":{},\"point_id\":{},\"point_index\":{},\"norm\":{},\"norm_bits\":{},\"frame_when_full_interest\":{},\"interest\":{},\"attempt_raw\":{},\"attempt_mod\":{},\"attempt_result\":{},\"insertion_raw\":{},\"insertion_index\":{},\"accumulator_before_bits\":{},\"accumulator_after_bits\":{}}}",
                        ctx.frame,
                        self.base.me,
                        optional_u32(ctx.original_creation_order),
                        phase4_attempts,
                        global.seek_points[idx].id,
                        idx,
                        square_norms[idx],
                        square_norms[idx].to_bits(),
                        global.seek_points[idx].frame_when_full_interest,
                        interest,
                        optional_u32(attempt_raw),
                        attempt,
                        accepted,
                        optional_u32(insertion_raw),
                        optional_usize(insertion_index),
                        accumulator_before_bits,
                        count_f.to_bits(),
                    );
                }
            }

            if debug_selection {
                eprintln!(
                    "SEEKAREA {{\"event\":\"selection_summary\",\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{:?},\"center\":[{},{}],\"standard_radius\":{},\"near_points\":{},\"expected_for_one\":{},\"visible_friends\":{},\"clears_help\":{},\"expected_before_help_random\":{},\"expected_points\":{},\"phase4_attempts\":{},\"phase4_accepts\":{},\"preselection_rng_draws\":{},\"phase4_rng_draws\":{},\"selection_rng_draws\":{},\"accepted_interest_sum\":{}}}",
                    ctx.frame,
                    self.base.me,
                    ctx.original_creation_order,
                    center.x,
                    center.y,
                    standard_radius,
                    near_sorted.len(),
                    expected_points_for_one,
                    tick.visible_seeking_friends,
                    tick.friend_seek_clears_help_flag,
                    expected_points_before_help_random,
                    expected_points,
                    phase4_attempts,
                    phase4_accepts,
                    preselection_rng_draws,
                    phase4_attempts + phase4_accepts,
                    preselection_rng_draws + phase4_attempts + phase4_accepts,
                    count_f,
                );
                eprintln!(
                    "SEEKAREA {{\"event\":\"selection_extra\",\"frame\":{},\"owner_creation_order\":{:?},\"flags\":{},\"seek_direction\":{},\"center_level\":{},\"obligatory\":{:?},\"obligatory2\":{:?},\"selected_random\":{:?}}}",
                    ctx.frame,
                    ctx.original_creation_order,
                    flags.bits(),
                    seek_direction,
                    center.level,
                    obligatory_idx.map(|i| global.seek_points[i].id),
                    obligatory2_idx.map(|i| global.seek_points[i].id),
                    selected_random
                        .iter()
                        .map(|&i| global.seek_points[i].id)
                        .collect::<Vec<_>>(),
                );
            }

            // ── Phase 5: reorder for optimal travel path ──
            for &idx in &selected_random {
                self.add_to_seek_point_list(idx, global);
            }

            // Add obligatory seek point at front. Insert with no
            // dedup — if the obligatory point was already added via
            // `add_to_seek_point_list`, it appears twice in the list
            // (and gets visited twice). Mirror that.
            if let Some(oblig_idx) = obligatory_idx {
                let id = global.seek_points[oblig_idx].id;
                self.my_seek_points.insert(0, id);
            }
        } else {
            // standard_radius == 0: only personal seek points
            debug_assert!(
                flags.intersects(SeekFlags::LOCATION_FIRST | SeekFlags::LOCATION_END),
                "SeekArea with radius 0 must have LOCATION_FIRST or LOCATION_END"
            );
        }

        // ── Phase 6: personal seek points (postprocessing) ──

        let debug_phase6 = seek_area_phase6_debug_matches(ctx.frame, ctx.original_creation_order);
        if debug_phase6 {
            eprintln!(
                "SEEKAREA {{\"event\":\"phase6_before\",\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{},\"state\":{},\"substate\":{},\"flags\":{},\"seek_direction\":{},\"list_size\":{},\"list_empty\":{},\"location_first\":{},\"location_end\":{},\"personal1_constructor\":\"{}\"}}",
                ctx.frame,
                self.base.me,
                ctx.original_creation_order
                    .expect("phase6 diagnostic matched an owner without creation order"),
                self.base.current_state as u32,
                self.base.current_substate as u32,
                flags.bits(),
                seek_direction,
                self.my_seek_points.len(),
                self.my_seek_points.is_empty(),
                flags.contains(SeekFlags::LOCATION_FIRST),
                flags.contains(SeekFlags::LOCATION_END),
                if !flags.contains(SeekFlags::LOCATION_FIRST) {
                    "none"
                } else if seek_direction == UNDEFINED_DIRECTION {
                    "position"
                } else {
                    "direction"
                },
            );
            eprintln!(
                "SEEKAREA {{\"event\":\"phase6_center\",\"frame\":{},\"owner_creation_order\":{:?},\"center\":[{},{}],\"seek_position\":[{},{}]}}",
                ctx.frame,
                ctx.original_creation_order,
                self.seek_center.x,
                self.seek_center.y,
                self.base.seek_position.x,
                self.base.seek_position.y,
            );
        }

        if flags.contains(SeekFlags::LOCATION_FIRST) {
            // FindDoorEnemyCouldBeBehind mutates seek_center in place.
            // Mirror that by copying the field out, mutating, then
            // writing back so any later reader of `seek_center` (e.g.
            // `EventReachPoint` handlers, `personal_seek_point_2`
            // below) sees the door-adjusted position.
            if flags.contains(SeekFlags::HOUSE) {
                let mut adjusted = self.seek_center;
                self.find_door_enemy_could_be_behind(
                    &mut adjusted,
                    seek_direction,
                    global,
                    ctx,
                    tick,
                );
                self.seek_center = adjusted;
            }

            let sp = if seek_direction != UNDEFINED_DIRECTION {
                let dir = SeekPointDirection {
                    position: self.seek_center,
                    direction: seek_direction,
                };
                let mut sp = SeekPoint::from_direction(&dir);
                sp.id = 1111;
                sp
            } else {
                let mut sp = SeekPoint::from_position(sim, self.seek_center);
                sp.id = 1111;
                sp
            };
            self.personal_seek_point_1 = Some(sp);
            self.my_seek_points.insert(0, 1111);
            if debug_phase6 {
                eprintln!(
                    "SEEKAREA {{\"event\":\"phase6_personal1\",\"frame\":{},\"owner_creation_order\":{},\"constructor\":\"{}\",\"inserted_id\":1111,\"list_size\":{}}}",
                    ctx.frame,
                    ctx.original_creation_order
                        .expect("phase6 diagnostic matched an owner without creation order"),
                    if seek_direction == UNDEFINED_DIRECTION {
                        "position"
                    } else {
                        "direction"
                    },
                    self.my_seek_points.len(),
                );
            }
        }

        let insert_personal2 =
            flags.contains(SeekFlags::LOCATION_END) || self.my_seek_points.is_empty();
        if insert_personal2 {
            // Create personal_seek_point_2 from the (possibly
            // door-adjusted) seek_center, not the original parameter.
            let mut sp = SeekPoint::from_position(sim, self.seek_center);
            sp.id = 2222;
            self.personal_seek_point_2 = Some(sp);
            self.my_seek_points.push(2222);
        }
        if debug_phase6 {
            eprintln!(
                "SEEKAREA {{\"event\":\"phase6_after\",\"frame\":{},\"owner_creation_order\":{},\"personal2_inserted\":{},\"personal2_constructor\":\"{}\",\"list_size\":{}}}",
                ctx.frame,
                ctx.original_creation_order
                    .expect("phase6 diagnostic matched an owner without creation order"),
                insert_personal2,
                if insert_personal2 { "position" } else { "none" },
                self.my_seek_points.len(),
            );
        }

        tracing::trace!(
            npc = self.base.me,
            frame = ctx.frame,
            seek_flags = ?self.seek_flags,
            list = ?self.my_seek_points,
            "SeekArea built its seek point list"
        );

        // Clear actual seek point (critical — missing caused memory
        // bugs).
        self.actual_seek_point = None;

        assert!(
            !self.my_seek_points.is_empty(),
            "SeekArea must produce at least one seek point"
        );

        if !ctx.in_building {
            self.seek_next_point(sim, global, ctx, tick);
        } else {
            // Inside a building: delay before seeking.
            self.seek_point_view_directions.clear();
            self.set_state(
                AiState::Seeking,
                Substate::SeekingSeekpointWatchingSidewards,
            );
            self.base.launch_timer(3, ctx.frame);
        }
    }

    /// Insert a seek point into `my_seek_points` at the position that
    /// minimises total travel distance.
    fn add_to_seek_point_list(&mut self, sp_idx: usize, global: &AiGlobalState) {
        let sp_id = global.seek_points[sp_idx].id;
        let sp_pos = global.seek_points[sp_idx].position;

        if self.my_seek_points.is_empty() {
            self.my_seek_points.push(sp_id);
            return;
        }

        let resolve_pos = |id: u16| -> Position {
            match id {
                1111 => self
                    .personal_seek_point_1
                    .as_ref()
                    .map(|s| s.position)
                    .unwrap_or(self.seek_center),
                2222 => self
                    .personal_seek_point_2
                    .as_ref()
                    .map(|s| s.position)
                    .unwrap_or(self.seek_center),
                _ => global
                    .seek_points
                    .get(id as usize)
                    .map(|s| s.position)
                    .unwrap_or(self.seek_center),
            }
        };

        // Try appending to the end
        let last_pos = resolve_pos(*self.my_seek_points.last().unwrap());
        let mut best_cost = pos_distance(sp_pos, last_pos);
        if sp_pos.level != last_pos.level {
            // Signed layer delta, not its magnitude: descending to a lower
            // layer makes appending *cheaper* here. The in-list insert cost
            // below uses a flat penalty instead, so the two are asymmetric.
            best_cost += parameters_ai::LAYER_CHANGE_PENALTY
                * (sp_pos.level as i32 - last_pos.level as i32) as f32;
        }
        let mut best_index = self.my_seek_points.len();

        // Try inserting between each pair (including before first)
        let mut prev_pos = self.seek_center;
        for (i, &id) in self.my_seek_points.iter().enumerate() {
            let next_pos = resolve_pos(id);
            // Cost of inserting sp between prev and next
            let mut cost = pos_distance(sp_pos, prev_pos) + pos_distance(next_pos, sp_pos)
                - pos_distance(next_pos, prev_pos);

            // Layer-change penalties
            if sp_pos.level != prev_pos.level {
                cost += 200.0;
            }
            if sp_pos.level != next_pos.level {
                cost += 200.0;
            }

            if cost < best_cost {
                best_cost = cost;
                best_index = i;
            }
            prev_pos = next_pos;
        }

        self.my_seek_points.insert(best_index, sp_id);
    }

    // -----------------------------------------------------------------------
    // SeekNextPoint — go to next seek point or return to duty
    // -----------------------------------------------------------------------

    /// Advance to the next seek point, or return to duty if none remain.
    /// Checks interest and lock state, skipping uninteresting or locked
    /// points.
    pub fn seek_next_point(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        let current_frame = ctx.frame;

        // Unlock the previous seek point
        if let Some(prev_id) = self.actual_seek_point.take()
            && let Some(sp) = resolve_seek_point_mut(
                prev_id,
                &mut self.personal_seek_point_1,
                &mut self.personal_seek_point_2,
                global,
            )
        {
            sp.locked = false;
        }

        self.current_task_priority = task_priority::SEEKING;

        // Strip NULL entries (safety against corrupt list).
        self.my_seek_points.retain(|&id| {
            resolve_seek_point_id(
                id,
                &self.personal_seek_point_1,
                &self.personal_seek_point_2,
                global,
            )
            .is_some()
        });

        // Check for beggars to examine. The reference gates only on
        // `beggars_to_control.size() > 0`; the adjacent assert is just
        // a sanity check that the previous beggar has been cleared,
        // not a guard on entry. The reset to 0 happens in the substate
        // exit path (mirroring the EVENT_DONE arm).
        if !self.beggars_to_control.is_empty() {
            debug_assert!(!self.beggars_to_control.contains(&self.beggar_to_examine));
            self.beggar_to_examine = self.beggars_to_control.pop().unwrap_or(0);
            // The beggar list mixes civilian profession-beggars (real)
            // and PCs in `Posture::SimulatingBeggar` (disguised). The
            // identification phases at
            // `SeekingSeekpointIdentifyingBeggar1/2` branch on
            // `beggar_is_npc` to either play the BEGGAR_SHOW_FACE
            // identify-and-resume sequence (real civilian) or commit
            // to combat (disguised PC), so commit the discriminator
            // here when the beggar is popped.
            self.beggar_is_npc = ctx
                .entity_view(self.beggar_to_examine)
                .map(|v| v.is_civilian())
                .unwrap_or(false);
            if let Some(pos) = self.positions_of_beggars_to_control.pop() {
                self.base.seek_position = pos;
                self.go_near(
                    AiState::Seeking,
                    Substate::SeekingSeekpointApproachingBeggar,
                    pos,
                    50,
                    GotoFlags::RUN,
                    ctx,
                );
                return;
            }
        }

        // No more seek points → return to duty
        if self.my_seek_points.is_empty() {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);

            // Say "ends search" if nothing alarming was found.
            let quiet_report = self.base.my_reconnaissance_report.report_type <= ReportType::Noise;
            let pending_followup = self
                .seek_flags
                .intersects(SeekFlags::REPORT_OFFICER_AFTER | SeekFlags::LOOK_FOR_HELP_AFTER);
            tracing::trace!(
                target: "robin_engine::ai_enemy::seek",
                frame = ctx.frame,
                me = self.base.me,
                report_type = ?self.base.my_reconnaissance_report.report_type,
                seek_flags = ?self.seek_flags,
                quiet_report,
                pending_followup,
                "SeekNextPoint: seek list exhausted"
            );
            if quiet_report && !pending_followup {
                self.base.say(Remark::EndsSearch);
            }
            return;
        }

        // Pop the next seek point
        let next_id = self.my_seek_points.remove(0);
        // Original assigns mpActualSeekPoint before testing the candidate.
        // When a locked or uninteresting point recurses into SeekNextPoint,
        // the recursive entry therefore unlocks that rejected candidate.
        // Preserve this seemingly odd global side effect: other investigators
        // can observe the lock release later in the same simulation frame.
        self.actual_seek_point = Some(next_id);

        // Check if locked or uninteresting — skip (recurse)
        let is_locked = {
            if let Some(sp) = resolve_seek_point_id(
                next_id,
                &self.personal_seek_point_1,
                &self.personal_seek_point_2,
                global,
            ) {
                sp.locked
            } else {
                // Invalid ID — skip
                self.seek_next_point(sim, global, ctx, tick);
                return;
            }
        };

        let debug_next_point = std::env::var_os("PARITY_DEBUG_SEEK_AREA_OWNER_POSITION").is_some();

        // Original short-circuits `IsLocked() || ...`: a locked candidate is
        // skipped without recalculating its shared interest or consuming the
        // acceptance draw. The recursive entry still unlocks it above.
        if is_locked {
            if debug_next_point {
                eprintln!(
                    "SEEKAREA {{\"event\":\"next_point_locked\",\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{:?},\"point_id\":{}}}",
                    ctx.frame, self.base.me, ctx.original_creation_order, next_id,
                );
            }
            self.seek_next_point(sim, global, ctx, tick);
            return;
        }

        // Recalculate interest
        let interest = resolve_seek_point_mut(
            next_id,
            &mut self.personal_seek_point_1,
            &mut self.personal_seek_point_2,
            global,
        )
        .unwrap_or_else(|| panic!("seek point {next_id} resolved immediately before mutation"))
        .calculate_interest(current_frame);

        let acceptance_roll =
            crate::sim_rng::u8(sim, crate::sim_rng::RngSite::SeekPointAcceptance, 0..100);
        if debug_next_point {
            eprintln!(
                "SEEKAREA {{\"event\":\"next_point_roll\",\"frame\":{},\"owner_handle\":{},\"owner_creation_order\":{:?},\"point_id\":{},\"interest\":{},\"roll\":{},\"accepted\":{},\"remaining\":{:?}}}",
                ctx.frame,
                self.base.me,
                ctx.original_creation_order,
                next_id,
                interest,
                acceptance_roll,
                acceptance_roll < interest,
                self.my_seek_points,
            );
        }
        if acceptance_roll >= interest {
            // Skip this point — try the next one
            self.seek_next_point(sim, global, ctx, tick);
            return;
        }

        // Subtract interest and lock this point
        if let Some(sp) = resolve_seek_point_mut(
            next_id,
            &mut self.personal_seek_point_1,
            &mut self.personal_seek_point_2,
            global,
        ) {
            sp.subtract_interest(
                parameters_ai::SEEK_POINT_EXAMINE_DELTA_INTEREST as u8,
                current_frame,
            );
            sp.locked = true;
        }

        // Get position and go there
        let seek_pos = resolve_seek_point_id(
            next_id,
            &self.personal_seek_point_1,
            &self.personal_seek_point_2,
            global,
        )
        .map(|sp| sp.position)
        .expect("seek point resolved successfully above");

        self.base.set_emoticon(EmoticonType::QuestionMark);

        let goto_flags = if self.seek_flags.contains(SeekFlags::WALKING) {
            GotoFlags::empty()
        } else {
            GotoFlags::RUN
        };
        self.go_to(
            AiState::Seeking,
            Substate::SeekingSeekpoint,
            seek_pos,
            goto_flags,
            ctx,
        );
    }

    // -----------------------------------------------------------------------
    // FindDoorEnemyCouldBeBehind
    // -----------------------------------------------------------------------

    /// When following an enemy that disappeared, check if they could
    /// have fled through a nearby building door. If so, teleport the
    /// seek center behind that door.
    fn find_door_enemy_could_be_behind(
        &self,
        seek_center: &mut Position,
        seek_direction: u16,
        global: &AiGlobalState,
        ctx: &AiContext,
        _tick: &AiPerTickData,
    ) {
        let mut min_distance = parameters_ai::MAX_SEARCH_ENEMY_BEHIND_DOOR_DISTANCE;
        let mut nearest_door: Option<&DoorSeekInfo> = None;

        for door_info in &global.door_seek_infos {
            if door_info.door_type != crate::gate::DoorType::Building {
                continue;
            }

            // Must be in the same sector as the seek center
            if Some(door_info.sector_out) != seek_center.sector.map(u16::from) {
                continue;
            }

            // Must not be the building we're already in.
            if ctx.in_building && Some(door_info.sector_in) == ctx.building_sector.map(u16::from) {
                continue;
            }

            // Complete the cached static authorization with the original's
            // two live gates: building capacity and rider state.
            let building = global
                .houses
                .iter()
                .find(|house| house.sector_index == u32::from(door_info.sector_in))
                .unwrap_or_else(|| {
                    panic!(
                        "building door {} targets sector {} without an AI house",
                        door_info.door_index, door_info.sector_in
                    )
                });
            if !door_info
                .is_npc_villain_authorized_direct(building.is_authorized(), ctx.self_is_rider)
            {
                continue;
            }

            let dx = door_info.point_out.x - seek_center.x;
            let dy = door_info.point_out.y - seek_center.y;

            // Check direction: door must be roughly in the seek direction
            let door_dir = vec_to_sector(dx, dy);
            let diff = (door_dir + 16 - seek_direction) & 15;
            if matches!(diff, 15 | 0 | 1) {
                let distance = (dx.abs().max(dy.abs())) as u16;
                if distance < min_distance {
                    min_distance = distance;
                    nearest_door = Some(door_info);
                }
            }
        }

        if let Some(door) = nearest_door {
            *seek_center = door.position_in;
        }
    }

    // -----------------------------------------------------------------------
    // DeadBodyAlert — corpse discovery triggers rank-dispatched alert
    // Port of the legacy corpse-discovery alert flow.
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn dead_body_alert(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        pos_center: Position,
        flags: SeekFlags,
        global: &mut AiGlobalState,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // Preamble: record the report regardless of rank.
        self.base
            .my_reconnaissance_report
            .update(ReportType::DeadBody, pos_center);

        let duty_radius = if self.soldier_profile_duty {
            parameters_ai::AI_SOD_DEAD_BODY_SEEK_RADIUS as u16
        } else {
            parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16
        };

        match self.get_rank() {
            ProfileRank::Soldier => {
                // A soldier with enough initiative (and not already
                // dispatched by an officer) searches the area themselves
                // before alerting anyone; otherwise alert the nearest
                // officer, and if none is found fall back to seeking
                // the area.
                if self.answer_question(Question::ShallISeekBeforeAlertingOfficer, ctx)
                    && self.base.antagonist == 0
                {
                    self.seek_area(
                        sim,
                        pos_center,
                        duty_radius,
                        SeekFlags::LOCATION_END
                            | SeekFlags::BODY_SEEK
                            | SeekFlags::LOOK_FOR_HELP_AFTER,
                        UNDEFINED_DIRECTION,
                        global,
                        ctx,
                        tick,
                    );
                } else {
                    let returns_to_instructed_group =
                        self.alert_officer_returns_to_instructed_group(tick);
                    let alerted = self.alert_officer(sim, pos_center, flags.bits(), ctx, tick);
                    if alerted && !returns_to_instructed_group {
                        // AlertOfficer calls GoNear synchronously, and Original
                        // inspects mbCouldntReachpoint before DeadBodyAlert
                        // returns. Rust constructs that route at the owner
                        // boundary, so close the actor prefix and resume the
                        // enclosing statement there.
                        self.base.outbox.reentrant.owner_work.push(
                            crate::ai::AiOwnerWork::ActorEffects(std::mem::take(
                                &mut self.base.outbox.actor,
                            )),
                        );
                        self.base
                            .outbox
                            .reentrant
                            .dead_body_alert_completion_pending = true;
                        self.base.outbox.reentrant.owner_work.push(
                            crate::ai::AiOwnerWork::ResumeDeadBodyAlertAfterAlertOfficer {
                                center: pos_center,
                                radius: duty_radius,
                            },
                        );
                    } else if !alerted {
                        self.seek_area(
                            sim,
                            pos_center,
                            duty_radius,
                            SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK,
                            UNDEFINED_DIRECTION,
                            global,
                            ctx,
                            tick,
                        );
                    }
                }
            }
            ProfileRank::Officer => {
                // Officer turns 180° (dir^8), then alerts nearby
                // soldiers with a BODY_SEEK flag, falling back to a
                // self-seek on failure. Note: pass the officer's own
                // position to AlertSoldiers, not `pos_center`.
                let new_dir = ctx.direction ^ 8;
                self.base.face_direction(new_dir, ctx);
                if !self.alert_soldiers(
                    ctx.position,
                    SeekFlags::BODY_SEEK.bits(),
                    global,
                    grid,
                    ctx,
                    tick,
                    AlertSoldiersFailureContinuation::SeekBody {
                        center: pos_center,
                        radius: duty_radius,
                    },
                ) {
                    self.seek_area(
                        sim,
                        pos_center,
                        duty_radius,
                        SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK,
                        UNDEFINED_DIRECTION,
                        global,
                        ctx,
                        tick,
                    );
                }
            }
            ProfileRank::Knight => {
                // Knights search their own vicinity.
                self.seek_area(
                    sim,
                    ctx.position,
                    duty_radius,
                    SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            }
            _ => {}
        }
    }

    /// Resume the statement following the soldier DeadBodyAlert call to
    /// AlertOfficer. A failed GoNear is consumed inline and falls back to the
    /// corpse search; a successful route has no further tail.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resume_dead_body_alert_after_alert_officer(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        center: Position,
        radius: u16,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        if !self.base.couldnt_reachpoint {
            return;
        }
        self.base.couldnt_reachpoint = false;
        self.seek_area(
            sim,
            center,
            radius,
            SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK,
            UNDEFINED_DIRECTION,
            global,
            ctx,
            tick,
        );
    }
    // -----------------------------------------------------------------------
    // Body examination
    // -----------------------------------------------------------------------

    /// Run to the nearest net covering a stuck victim and prepare to
    /// remove it.
    ///
    /// Picks the covering net with minimum `MaxNormDistance` from self,
    /// records the chosen net in `interesting_object` (so
    /// `SeekingTakingNet` drives the SEARCH+TAKE sequence against the
    /// right net), and routes either to the net (reachable) or to the
    /// victim (emergency fallback) depending on
    /// `IsStraightMovementAutorized`.
    pub fn run_to_free_net_victim(
        &mut self,
        victim: HumanHandle,
        ctx: &AiContext,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        let Some(view) = ctx.entity_view(victim) else {
            tracing::warn!(
                me = self.base.me,
                victim,
                "run_to_free_net_victim: victim not in entity view map"
            );
            return;
        };

        // victim.ComputeNetsCoveringMe(list_nets)
        // (reverse-index the net → victims map) then pick the minimum-
        // `MaxNormDistance` net.  The reverse index lives on the view as
        // `covering_nets`, pre-scanned by `build_entity_views`.
        // `MaxNormDistance` stretches Y by `INVERSE_ASPECT_RATIO` before
        // the Chebyshev max.
        let my_pos = ctx.position;
        let mut nearest: Option<crate::ai_entity_view::NetCoverInfo> = None;
        let mut min_dist = f32::INFINITY;
        for net in &view.covering_nets {
            let dx = (net.position.x - my_pos.x).abs();
            let dy = (net.position.y - my_pos.y).abs() * INVERSE_ASPECT_RATIO;
            let dist = dx.max(dy);
            if dist < min_dist {
                min_dist = dist;
                nearest = Some(*net);
            }
        }
        let Some(net) = nearest else {
            // Asserts `list_nets.size() > 0` and
            // `pNearestNet != NULL` (line 18637).  Reaching here means
            // `stuck_under_net` was true but no covering net survived
            // the pre-scan — e.g. a race between `unapply_net_effect`
            // and the view builder.  Log and bail without corrupting
            // AI state rather than asserting.
            tracing::warn!(
                me = self.base.me,
                victim,
                "run_to_free_net_victim: stuck victim has no covering nets in view"
            );
            return;
        };

        // Record both the victim and the chosen net.
        self.base.detected_body = victim;
        self.base.interesting_object = net.handle;

        // If the victim → net segment is clear on the
        // victim's layer for my move-box, walk up to the net and stop
        // at `GetRadius() + 15`.  Otherwise fall back to the victim's
        // position with stop distance 15.
        let victim_pos = view.position;
        let net_pos = net.position;
        let grid = grid.unwrap_or(&ctx.fast_grid);
        let reachable = grid.is_straight_movement_authorized(
            crate::coordinates::MapPoint::new(victim_pos.x, victim_pos.y),
            crate::coordinates::MapPoint::new(net_pos.x, net_pos.y),
            victim_pos.level,
            &ctx.move_box,
        );
        let (pos_goal, distance) = if reachable {
            (net_pos, (net.radius as i32) + 15)
        } else {
            (victim_pos, 15)
        };

        // SetState(Seeking, SeekingNet); GoNear(...,
        // GOTO_RUN); LaunchTimer(10).  `go_near` folds the SetState in.
        self.go_near(
            AiState::Seeking,
            Substate::SeekingNet,
            pos_goal,
            distance,
            GotoFlags::RUN,
            ctx,
        );
        self.base.launch_timer(10, ctx.frame);
    }

    /// SearchCharly.
    /// Begins a sweep of the checkpoint charly's patrol path:
    ///
    /// * Officers re-enter [`Substate::SeekingCharlyWatching`] and let
    ///   the existing `MissedCharlyAlert` flow run.
    /// * Soldiers / knights say `MissesCharly`, transition to
    ///   [`Substate::SeekingCharly`], and rebuild
    ///   [`Self::search_charly_way`] from the charly's hiking path —
    ///   nearest waypoint first, with the "skip a >90° pivot"
    ///   nudge, then wrap around to enumerate the rest.  When the
    ///   charly has no patrol path, the way is seeded with the
    ///   charly's `initial_position`.
    ///
    /// Multi-waypoint sweeps run with `RUN | DONT_STOP` so the seeker
    /// chains waypoints without halting between them.
    pub fn search_charly(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        self.base.set_emoticon(EmoticonType::QuestionMark);

        // Officer arm.
        if self.get_rank() == ProfileRank::Officer {
            self.set_state(AiState::Seeking, Substate::SeekingCharlyWatching);
            self.base.fire_self_stimulus(StimulusType::EventDone);
            return;
        }

        // Soldier/knight prelude.
        self.base.say(Remark::MissesCharly);
        self.search_charly_way.clear();
        self.base.macro_in_progress = false;
        self.current_task_priority = task_priority::MISSED_FRIEND;
        self.seeking_charly = true;

        // No checkpoint → ReturnToDuty.
        if self.base.checkpoint_charly == 0 {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            return;
        }
        let Some(view) = ctx.entity_view(self.base.checkpoint_charly) else {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            return;
        };

        // Build the search way.
        let my_pos = ctx.position;
        let waypoints: Vec<Position> = match (view.has_patrol_path, view.patrol_hiking_path_index) {
            (true, Some(path_index)) => {
                // Read the hiking path's waypoint list off the AI's
                // shared `hiking_paths` ref.  The charly may share the
                // same engine-wide `Arc<Vec<RawHikingPath>>` as us.
                let raw = ctx.hiking_paths.get(usize::from(path_index)).cloned();
                if let Some(path) = raw {
                    let n = path.waypoints.len();
                    if n == 0 {
                        Vec::new()
                    } else {
                        // Build positions from RawWaypoint.
                        let pos_list: Vec<Position> = path
                            .waypoints
                            .iter()
                            .map(|w| Position {
                                x: w.x as f32,
                                y: w.y as f32,
                                sector: None,
                                level: w.level,
                            })
                            .collect();
                        // Nearest waypoint by MaxNorm.
                        let mut best_idx = 0usize;
                        let mut best_dist = f32::INFINITY;
                        for (i, p) in pos_list.iter().enumerate() {
                            let dx = (p.x - my_pos.x).abs();
                            let dy = (p.y - my_pos.y).abs();
                            let d = dx.max(dy);
                            if d < best_dist {
                                best_dist = d;
                                best_idx = i;
                            }
                        }
                        // Pivot-skip — if the turn
                        // from `posThis` (best) to the next waypoint
                        // exceeds 90°, advance to that next waypoint.
                        let next_idx = (best_idx + 1) % n;
                        let pos_this = pos_list[best_idx];
                        let pos_next = pos_list[next_idx];
                        let v1x = pos_this.x - my_pos.x;
                        let v1y = pos_this.y - my_pos.y;
                        let v2x = pos_next.x - pos_this.x;
                        let v2y = pos_next.y - pos_this.y;
                        let dot = v1x * v2x + v1y * v2y;
                        let start_idx = if dot < 0.0 { next_idx } else { best_idx };
                        // Enumerate all waypoints
                        // beginning at `start_idx`, wrapping around.
                        (0..n).map(|i| pos_list[(start_idx + i) % n]).collect()
                    }
                } else {
                    // No hiking-path data available — fall back to the
                    // charly's live position.
                    vec![view.position]
                }
            }
            // No path → seed from initial position.
            _ => vec![view.initial_position],
        };

        if waypoints.is_empty() {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            return;
        }

        // Stash the way and kick off the seek.
        self.search_charly_way = waypoints;
        self.set_state(AiState::Seeking, Substate::SeekingCharly);
        self.set_alert_status(AlertLevel::Yellow);
        // GOTO_RUN | GOTO_DONTSTOP when the way has
        // more than one waypoint so we don't halt between them.
        let first = self.search_charly_way[0];
        let flags = if self.search_charly_way.len() > 1 {
            crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::DONT_STOP
        } else {
            crate::ai::GotoFlags::RUN
        };
        self.base.go_to(first, flags, ctx);
    }

    pub fn run_to_examine_body(
        &mut self,
        body: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        // RunToExamineBody: if stuck under a net, delegate to
        // `RunToFreeNetVictim`; else focus, mark X
        // emoticon, and run up to the body.
        let view = ctx.entity_view(body);
        let stuck = view.map(|v| v.stuck_under_net).unwrap_or(false);
        if stuck {
            // RunToFreeNetVictim(body).
            self.run_to_free_net_victim(body, ctx, grid);
            return;
        }

        self.base.detected_body = body;
        // seek_position = Position(body). Prefer the live entity view
        // (covers bodies that aren't in the fighter snapshot), then the
        // fighter snapshot. The original dereferences the body here, so a
        // missing required body cannot become a fabricated map origin.
        self.base.seek_position = view
            .map(|v| v.position)
            .or_else(|| self.find_fighter(body, tick).map(|f| f.position))
            .unwrap_or_else(|| {
                panic!(
                    "soldier {} cannot examine missing body {}",
                    self.base.me, body
                )
            });
        // SetEmoticon(EMOTICON_X_MARK).
        self.base.set_emoticon(EmoticonType::XMark);
        // SetState(STATE_SEEKING, SUBSTATE_SEEKING_BODY).
        // Matched implicitly by `go_near` below.
        // Focus(body).
        self.base.outbox.actor.set_focus(body);
        self.go_near(
            AiState::Seeking,
            Substate::SeekingBody,
            self.base.seek_position,
            parameters_ai::AI_STOP_BEFORE_BODY_STEPS,
            GotoFlags::RUN,
            ctx,
        );
        self.base.launch_timer(10, ctx.frame);
    }

    /// Check the queue of other bodies previously seen; if one is
    /// still out-of-order, run to examine it and return `true`.
    /// Otherwise clear the queue (bodies that recovered get skipped)
    /// and return `false`.
    /// Legacy examine-other-bodies behavior.
    pub fn examine_other_bodies(&mut self, ctx: &AiContext, tick: &AiPerTickData) -> bool {
        // Prune from the front while the first body
        // `IsOutOfOrder() == false` (i.e. has recovered / woken up).
        while let Some(&first) = self.other_bodies_to_examine.first() {
            // Body queues deliberately contain out-of-order humans. The
            // tactical nearby-fighter list filters those actors out before AI
            // dispatch, so absence there does not mean the body recovered.
            // Use the complete handle-indexed entity snapshot, matching the
            // Original's direct `IsOutOfOrder()` call
            // (`RHartificialmalignity.cpp:20128`). `IsAbleToFight` is a
            // different predicate and must not stand in for it: civilians
            // never report able-to-fight, so a woken civilian sleeper would
            // stay queued forever and get re-examined on the spot.
            let still_down = ctx
                .entity_view(first)
                .unwrap_or_else(|| {
                    panic!(
                        "soldier {} cannot prune missing queued body {first}",
                        self.base.me
                    )
                })
                .is_out_of_order();
            if still_down {
                break;
            }
            self.other_bodies_to_examine.remove(0);
        }
        let Some(&body) = self.other_bodies_to_examine.first() else {
            return false;
        };
        self.other_bodies_to_examine.remove(0);
        self.run_to_examine_body(body, ctx, tick, None);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_enemy::CampSoldierInfo;

    fn alert_test_officer(handle: u32, substate: Substate) -> CampSoldierInfo {
        let position = Position {
            x: 80.0,
            y: 40.0,
            ..Position::default()
        };
        CampSoldierInfo {
            handle,
            active: true,
            position,
            position_world: crate::coordinates::WorldPoint3D::new(80.0, 40.0, 0.0),
            direction: 0,
            rank: ProfileRank::Officer,
            ai_state: AiState::Default,
            ai_substate: substate,
            is_able_to_fight: true,
            is_dead: false,
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
            duty_flag: false,
            is_tower_guard: false,
            company_number: 0,
            in_building: false,
            forecast_destination: Some(crate::ai::PreparedForecastDestination::fixed(position, 0)),
            detectable_bodies: Vec::new(),
            seek_position: Position::default(),
            current_task_priority: 0,
            minimal_task_priority: 0,
            view_direction: [1.0, 0.0],
            view_radius: 500,
            real_half_aperture: 1.0,
            eye_blind: false,
        }
    }

    #[test]
    fn dead_body_alert_with_officer_queues_actor_prefix_then_typed_resume() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(40);
        ai.company_number = 0;
        ai.base.antagonist = 99;
        let center = Position {
            x: 10.0,
            y: 20.0,
            ..Position::default()
        };
        let ctx = AiContext {
            camp: crate::element::Camp::Lacklandists,
            in_building: true,
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.camp_soldiers
            .push(alert_test_officer(7, Substate::DefaultGotoPost));

        ai.dead_body_alert(
            &sim,
            center,
            SeekFlags::empty(),
            &mut AiGlobalState::default(),
            None,
            &ctx,
            &tick,
        );

        assert!(ai.base.outbox.reentrant.dead_body_alert_completion_pending);
        assert_eq!(ai.base.outbox.reentrant.owner_work.len(), 3);
        assert!(matches!(
            ai.base.outbox.reentrant.owner_work[0],
            crate::ai::AiOwnerWork::StateChange(_)
        ));
        assert!(matches!(
            ai.base.outbox.reentrant.owner_work[1],
            crate::ai::AiOwnerWork::ActorEffects(_)
        ));
        assert!(matches!(
            ai.base.outbox.reentrant.owner_work[2],
            crate::ai::AiOwnerWork::ResumeDeadBodyAlertAfterAlertOfficer {
                center: queued_center,
                radius: 300
            } if queued_center == center
        ));
        assert!(ai.base.outbox.actor.orders.is_empty());

        // Settle the queued GoNear as a route failure, then run the exact
        // typed tail that owns that result.
        ai.base.couldnt_reachpoint = true;
        ai.base.outbox.reentrant.dead_body_alert_completion_pending = false;
        ai.resume_dead_body_alert_after_alert_officer(
            &sim,
            center,
            300,
            &mut AiGlobalState::default(),
            &ctx,
            &tick,
        );
        assert_eq!(
            ai.seek_flags,
            SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK
        );
        assert!(ai.personal_seek_point_2.is_some());
        assert!(ai.base.outbox.reentrant.self_stimuli.is_empty());
    }

    #[test]
    fn dead_body_alert_instructed_group_route_never_queues_fallback() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(44);
        ai.company_number = 0;
        ai.base.antagonist = 7;
        ai.seek_flags = SeekFlags::REPORT_OFFICER_AFTER;
        let ctx = AiContext {
            camp: crate::element::Camp::Lacklandists,
            in_building: true,
            ..AiContext::default()
        };
        let mut tick = AiPerTickData::stub();
        tick.camp_soldiers.push(alert_test_officer(
            7,
            Substate::SeekingOfficerWaitForInstructedGroup,
        ));

        ai.dead_body_alert(
            &sim,
            Position::default(),
            SeekFlags::empty(),
            &mut AiGlobalState::default(),
            None,
            &ctx,
            &tick,
        );

        // Route construction reports failure only after the AI borrow is
        // released. With no typed tail queued, that later result cannot run
        // the ordinary DeadBodyAlert fallback.
        ai.base.couldnt_reachpoint = true;
        assert!(!ai.base.outbox.reentrant.dead_body_alert_completion_pending);
        assert!(
            !ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .any(|work| matches!(
                    work,
                    crate::ai::AiOwnerWork::ResumeDeadBodyAlertAfterAlertOfficer { .. }
                ))
        );
        assert!(ai.personal_seek_point_2.is_none());
        assert!(!ai.seek_flags.contains(SeekFlags::BODY_SEEK));
        assert_eq!(
            ai.base.current_substate,
            Substate::SeekingSoldierReturnToOfficer
        );
    }

    #[test]
    fn failed_dead_body_alert_officer_route_falls_back_to_body_seek() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(41);
        ai.company_number = 0;
        ai.base.couldnt_reachpoint = true;
        let center = Position {
            x: 120.0,
            y: 240.0,
            ..Position::default()
        };
        let ctx = AiContext {
            camp: crate::element::Camp::Lacklandists,
            in_building: true,
            ..AiContext::default()
        };

        ai.resume_dead_body_alert_after_alert_officer(
            &sim,
            center,
            300,
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
        );

        assert!(!ai.base.couldnt_reachpoint);
        assert_eq!(
            ai.seek_flags,
            SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK
        );
        assert_eq!(ai.my_seek_points, vec![2222]);
        assert_eq!(
            ai.personal_seek_point_2
                .as_ref()
                .expect("body search owns its personal endpoint")
                .position,
            center
        );
        assert!(ai.base.outbox.reentrant.self_stimuli.is_empty());
    }

    #[test]
    fn successful_dead_body_alert_officer_route_has_no_fallback() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(42);
        ai.company_number = 0;
        let ctx = AiContext {
            camp: crate::element::Camp::Lacklandists,
            in_building: true,
            ..AiContext::default()
        };

        ai.resume_dead_body_alert_after_alert_officer(
            &sim,
            Position::default(),
            300,
            &mut AiGlobalState::default(),
            &ctx,
            &AiPerTickData::stub(),
        );

        assert!(ai.seek_flags.is_empty());
        assert!(ai.my_seek_points.is_empty());
        assert!(ai.personal_seek_point_2.is_none());
        assert!(ai.base.outbox.actor.orders.is_empty());
    }

    #[test]
    fn dead_body_alert_without_officer_falls_back_immediately() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(43);
        ai.company_number = 0;
        // Force the AlertOfficer arm independently of the random answer.
        ai.base.antagonist = 99;
        let center = Position {
            x: 60.0,
            y: 90.0,
            ..Position::default()
        };
        let ctx = AiContext {
            camp: crate::element::Camp::Lacklandists,
            in_building: true,
            ..AiContext::default()
        };

        ai.dead_body_alert(
            &sim,
            center,
            SeekFlags::empty(),
            &mut AiGlobalState::default(),
            None,
            &ctx,
            &AiPerTickData::stub(),
        );

        assert_eq!(
            ai.seek_flags,
            SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK
        );
        assert_eq!(ai.my_seek_points, vec![2222]);
        assert!(ai.personal_seek_point_2.is_some());
        assert!(!ai.base.outbox.reentrant.dead_body_alert_completion_pending);
        assert!(
            !ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .any(|work| matches!(
                    work,
                    crate::ai::AiOwnerWork::ResumeDeadBodyAlertAfterAlertOfficer { .. }
                ))
        );
    }

    #[test]
    fn seek_point_interest_accumulator_narrows_once_after_double_arithmetic() {
        let accumulated = accumulate_seek_point_interest(0.0, 10);
        let all_f32 = 10.0_f32 * 0.01_f32;

        assert_eq!(accumulated.to_bits(), 0x3dcc_cccd);
        assert_eq!(all_f32.to_bits(), 0x3dcc_cccc);
    }

    #[test]
    fn seek_point_interest_accumulator_preserves_original_threshold_crossing() {
        let interests = [55, 19, 32, 44, 1, 44, 27, 97, 56, 15, 83, 26, 14, 0, 56, 31];
        let accumulated = interests
            .into_iter()
            .fold(0.0, accumulate_seek_point_interest);
        let all_f32 = interests.into_iter().fold(0.0_f32, |current, interest| {
            current + f32::from(interest) * 0.01_f32
        });

        assert_eq!(accumulated.to_bits(), 0x40c0_0001);
        assert_eq!(all_f32.to_bits(), 0x40bf_ffff);
        assert!(accumulated >= 6.0);
        assert!(all_f32 < 6.0);
    }

    #[test]
    fn seek_point_interest_accumulator_keeps_exact_threshold_control() {
        let accumulated = [50, 50]
            .into_iter()
            .fold(0.0, accumulate_seek_point_interest);

        assert_eq!(accumulated, 1.0);
    }

    #[test]
    fn seek_area_obligatory_selection_respects_original_finite_sentinel() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(131);
        let center = Position {
            x: 1585.620_1,
            y: 2454.293_2,
            sector: None,
            level: 0,
        };
        let mut global = AiGlobalState::default();
        global.seek_points = vec![
            SeekPoint {
                position: Position {
                    x: 1547.0,
                    y: 2488.0,
                    sector: None,
                    level: 0,
                },
                frame_when_full_interest: 0,
                directions: vec![],
                last_calculated_interest: 100,
                locked: false,
                id: 212,
            },
            SeekPoint {
                position: Position {
                    x: 1753.0,
                    y: 2670.0,
                    sector: None,
                    level: 0,
                },
                frame_when_full_interest: 0,
                directions: vec![],
                last_calculated_interest: 100,
                locked: false,
                id: 218,
            },
        ];
        let ctx = AiContext {
            camp: crate::element::Camp::Lacklandists,
            in_building: true,
            ..AiContext::default()
        };

        ai.seek_area(
            &sim,
            center,
            300,
            SeekFlags::empty(),
            8,
            &mut global,
            &ctx,
            &AiPerTickData::stub(),
        );

        assert_eq!(ai.my_seek_points.first(), Some(&212));
    }

    #[test]
    fn seek_next_point_preserves_the_search_center() {
        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(118);
        let search_center = Position {
            x: 1397.772_9,
            y: 1864.478_5,
            sector: None,
            level: 0,
        };
        let route_point = Position {
            x: 1236.0,
            y: 1589.0,
            sector: None,
            level: 8,
        };
        ai.base.seek_position = search_center;
        ai.my_seek_points.push(1111);
        ai.personal_seek_point_1 = Some(SeekPoint {
            position: route_point,
            frame_when_full_interest: 0,
            directions: vec![4, 10, 15],
            last_calculated_interest: 100,
            locked: false,
            id: 1111,
        });

        ai.seek_next_point(
            &sim,
            &mut AiGlobalState::default(),
            &AiContext::default(),
            &AiPerTickData::stub(),
        );

        assert_eq!(ai.base.last_goto_destination, route_point);
        assert_eq!(ai.base.seek_position, search_center);
    }

    #[test]
    fn locked_seek_point_skips_interest_recalculation_and_acceptance_draw() {
        use crate::sim_rng::{RngSite, with_draw_trace};

        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(118);
        ai.my_seek_points = vec![0, 1];
        let locked_position = Position {
            x: 100.0,
            ..Position::default()
        };
        let accepted_position = Position {
            x: 200.0,
            ..Position::default()
        };
        let mut global = AiGlobalState::default();
        global.seek_points = vec![
            SeekPoint {
                position: locked_position,
                frame_when_full_interest: 1_000,
                directions: vec![2],
                last_calculated_interest: 7,
                locked: true,
                id: 0,
            },
            SeekPoint {
                position: accepted_position,
                frame_when_full_interest: 0,
                directions: vec![4],
                last_calculated_interest: 3,
                locked: false,
                id: 1,
            },
        ];
        let ctx = AiContext {
            frame: 500,
            ..AiContext::default()
        };

        let (_, draws) = with_draw_trace(|| {
            ai.seek_next_point(&sim, &mut global, &ctx, &AiPerTickData::stub());
        });

        assert_eq!(draws, [RngSite::SeekPointAcceptance]);
        assert_eq!(global.seek_points[0].last_calculated_interest, 7);
        assert!(!global.seek_points[0].locked);
        assert_eq!(global.seek_points[1].last_calculated_interest, 100);
        assert!(global.seek_points[1].locked);
        assert_eq!(ai.actual_seek_point, Some(1));
        assert_eq!(ai.base.last_goto_destination, accepted_position);
    }

    #[test]
    fn unlocked_seek_point_recalculates_draws_subtracts_and_locks() {
        use crate::sim_rng::{RngSite, with_draw_trace};

        let sim = crate::sim_rng::test_context();
        let mut ai = EnemyAi::new(118);
        ai.my_seek_points.push(0);
        let destination = Position {
            x: 300.0,
            ..Position::default()
        };
        let mut global = AiGlobalState::default();
        global.seek_points.push(SeekPoint {
            position: destination,
            frame_when_full_interest: 501,
            directions: vec![6],
            last_calculated_interest: 7,
            locked: false,
            id: 0,
        });
        let ctx = AiContext {
            frame: 500,
            ..AiContext::default()
        };

        let (_, draws) = with_draw_trace(|| {
            ai.seek_next_point(&sim, &mut global, &ctx, &AiPerTickData::stub());
        });

        assert_eq!(draws, [RngSite::SeekPointAcceptance]);
        assert_eq!(global.seek_points[0].last_calculated_interest, 100);
        assert_eq!(global.seek_points[0].frame_when_full_interest, 5_501);
        assert!(global.seek_points[0].locked);
        assert_eq!(ai.actual_seek_point, Some(0));
        assert_eq!(ai.base.last_goto_destination, destination);
    }
}
