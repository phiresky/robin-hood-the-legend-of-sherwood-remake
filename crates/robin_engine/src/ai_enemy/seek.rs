//! Seek-and-search behaviours: the area-search seek-point loop, body
//! examination, search-for-charly, run-to-free-net-victim,
//! find-door-enemy-could-be-behind, dead-body-alert dispatch, and the
//! flee primitive.

use crate::ai::*;
use crate::parameters_ai;
use crate::sim_rng::SimulationContext;

use super::ThinkEnv;

fn seek_area_owner_position_debug_gate() -> &'static crate::engine::diagnostics::ParityGate<2> {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<2>> = std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_SEEK_AREA_OWNER_POSITION",
            [
                "PARITY_DEBUG_SEEK_AREA_FRAME",
                "PARITY_DEBUG_SEEK_AREA_CREATION_ORDER",
            ],
        )
    })
}

fn seek_area_selection_debug_matches(frame: u32, creation_order: Option<u32>) -> bool {
    let gate = seek_area_owner_position_debug_gate();
    gate.enabled() && gate.matches_required([Some(frame), creation_order])
}

/// Phase-6 diagnostics share the owner-position filters but require both.
fn seek_area_phase6_debug_gate() -> &'static crate::engine::diagnostics::ParityGate<2> {
    static GATE: std::sync::OnceLock<crate::engine::diagnostics::ParityGate<2>> =
        std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        crate::ai::parity_gate::required_parity_gate(
            "PARITY_DEBUG_SEEK_AREA_PHASE6",
            [
                "PARITY_DEBUG_SEEK_AREA_FRAME",
                "PARITY_DEBUG_SEEK_AREA_CREATION_ORDER",
            ],
        )
    })
}

fn seek_area_phase6_debug_enabled() -> bool {
    seek_area_phase6_debug_gate().enabled()
}

fn seek_area_phase6_debug_matches(frame: u32, creation_order: Option<u32>) -> bool {
    seek_area_phase6_debug_gate().matches_required([Some(frame), creation_order])
}

#[inline]
fn accumulate_seek_point_interest(current: f32, interest: u8) -> f32 {
    (f64::from(current) + f64::from(interest) * 0.01_f64) as f32
}

#[inline]
fn legacy_seek_direction_delta(direction: u16, seek_direction: u16) -> u16 {
    direction.wrapping_add(16).wrapping_sub(seek_direction)
}

use super::util::{pos_distance, resolve_seek_point_id, resolve_seek_point_mut, vec_to_sector};
use super::{
    AlertSoldiersFailureContinuation, EnemyAi, ProfileRank, SeekFlags, UNDEFINED_DIRECTION,
    task_priority,
};

/// Immutable inputs shared by the candidate and personal-point phases.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct SeekAreaSpec {
    pub(crate) center: Position,
    pub(crate) standard_radius: u16,
    pub(crate) flags: SeekFlags,
    pub(crate) seek_direction: u16,
}

/// Candidate indices retain global-array order for equal distances. In particular,
/// obligatory indices remain separate: their later insertion deliberately allows duplicates.
#[derive(serde::Serialize, serde::Deserialize)]
struct SeekAreaCandidates {
    square_norms: Vec<f32>,
    near_sorted: Vec<usize>,
    obligatory_idx: Option<usize>,
    obligatory2_idx: Option<usize>,
    expected_points_for_one: u16,
}

impl SeekAreaCandidates {
    fn new(spec: SeekAreaSpec, global: &AiGlobalState) -> Self {
        let SeekAreaSpec {
            center,
            standard_radius,
            seek_direction,
            ..
        } = spec;
        let sq_standard_radius = (standard_radius as f32) * (standard_radius as f32);
        let mut obligatory_idx: Option<usize> = None;
        let mut obligatory2_idx: Option<usize> = None;
        // The original game seeds both single-precision minima with its infinity sentinel
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
            // The original game uses modulo 15 rather than wrapping at 16,
            // making case 15 unreachable — port the bug literally
            // so sector-bucket assignments match for boundary
            // sectors (e.g. seek_direction=0, sector 14:
            // (14+16)%15 = 0 → "in direction"; & 15 would give 14
            // → "almost").
            if seek_direction != UNDEFINED_DIRECTION {
                let dir_sector = vec_to_sector(dx, dy);
                // All operands are original-game unsigned 16-bit values. Preserve their
                // unsigned wrap when a script supplies a direction above
                // `dir_sector + 16`; debug builds must not turn that
                // defined legacy arithmetic into an overflow panic.
                let diff = legacy_seek_direction_delta(dir_sector, seek_direction) % 15;
                match diff {
                    15 | 0 | 1
                        if square_norm < min_sqr_norm && sp.position.level == center.level =>
                    {
                        obligatory_idx = Some(i);
                        min_sqr_norm = square_norm;
                    }
                    14 | 2 if square_norm < min_sqr_norm2 && sp.position.level == center.level => {
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

        Self {
            square_norms,
            near_sorted,
            obligatory_idx,
            obligatory2_idx,
            expected_points_for_one,
        }
    }
}

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

    pub fn flee(&mut self, danger_pos: &Position, ctx: &AiContext, global: &AiGlobalState) {
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
                let dist = danger_to_sp.0.abs().max(danger_to_sp.1.abs()); // Maximum norm
                if dist > max_distance {
                    max_distance = dist;
                    best_point = Some(sp.position);
                }
            }
        }

        let Some(flee_pos) = best_point else {
            // No eligible destination: warn and preserve the current state
            // instead of inventing a flee destination.
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
    // Search the area after losing sight of an enemy
    // -----------------------------------------------------------------------

    /// Begin a search pattern around `center`. Selects seek points from
    /// the global array based on distance, interest, and direction, then
    /// visits them in an optimised order.
    pub(crate) fn seek_area(
        &mut self,
        _env: ThinkEnv<'_>,
        center: Position,
        standard_radius: u16,
        flags: SeekFlags,
        seek_direction: u16,
        _global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<()> {
        Err(crate::ai::DutyCall {
            flags: DutyFlags::empty(),
            think_result: false,
            tail: crate::ai::DutyTail::SeekArea {
                center,
                standard_radius,
                flags,
                seek_direction,
            },
            after: Vec::new(),
        })
    }

    pub(crate) fn append_global_area_seek_points(
        &mut self,
        sim: &SimulationContext,
        frame: u32,
        creation_order: Option<u32>,
        seeking_friends: usize,
        clears_help: bool,
        spec: SeekAreaSpec,
        global: &mut AiGlobalState,
    ) {
        let candidates = SeekAreaCandidates::new(spec, global);
        let square_norms = &candidates.square_norms;
        let near_sorted = &candidates.near_sorted;
        let obligatory_idx = candidates.obligatory_idx;
        let center = spec.center;
        let current_frame = frame;
        if seek_area_selection_debug_matches(frame, creation_order) {
            for (i, sp) in global.seek_points.iter().enumerate() {
                crate::ai_enemy::parity_trace::SeekareaPointDump {
                    frame: &(frame),
                    index: &(i),
                    id: &(sp.id),
                    x: &(sp.position.x),
                    y: &(sp.position.y),
                    level: &(sp.position.level),
                    center_x: &(center.x),
                    center_y: &(center.y),
                    center_level: &(center.level),
                    norm: &(square_norms[i]),
                    norm_bits: &(square_norms[i].to_bits()),
                    near: &(near_sorted.contains(&i)),
                    frame_when_full_interest: &(sp.frame_when_full_interest),
                }
                .emit();
            }
        }

        // If nearest point was recently examined, don't look for help
        if let Some(&first_idx) = near_sorted.first()
            && global.seek_points[first_idx].calculate_interest(current_frame) < 90
        {
            self.seek_flags &= !SeekFlags::LOOK_FOR_HELP_AFTER;
        }

        let selected_random = self.select_area_seek_points(
            sim,
            frame,
            creation_order,
            seeking_friends,
            clears_help,
            spec,
            &candidates,
            global,
        );
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
    }

    fn select_area_seek_points(
        &mut self,
        sim: &SimulationContext,
        frame: u32,
        creation_order: Option<u32>,
        seeking_friends: usize,
        clears_help: bool,
        spec: SeekAreaSpec,
        candidates: &SeekAreaCandidates,
        global: &mut AiGlobalState,
    ) -> Vec<usize> {
        let SeekAreaSpec {
            center,
            standard_radius,
            flags,
            seek_direction,
        } = spec;
        let square_norms = &candidates.square_norms;
        let near_sorted = &candidates.near_sorted;
        let obligatory_idx = candidates.obligatory_idx;
        let obligatory2_idx = candidates.obligatory2_idx;
        let expected_points_for_one = candidates.expected_points_for_one;
        let current_frame = frame;
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
        for _ in 0..seeking_friends {
            friend_factor *= parameters_ai::SEEK_POINT_NUMBER_FACTOR;
        }
        if clears_help {
            self.seek_flags &= !SeekFlags::LOOK_FOR_HELP_AFTER;
        }

        let mut expected_points = (expected_points_for_one as f32 * friend_factor) as u16;
        let expected_points_before_help_random = expected_points;
        let mut preselection_rng_draws = 0usize;

        if self.seek_flags.contains(SeekFlags::LOOK_FOR_HELP_AFTER) {
            // Reduce seek count when planning to ask for help.
            // The original game's courage consideration contributes
            // neither value nor weight here. Its rectangular distribution
            // is a plain `min + rand() % range` and ignores consideration
            // scores, so courage does not bias this sample. Rust's uniform
            // sample matches. The courage axis itself *is* implemented
            // (`AiBrain::soldier_profile_courage` / `get_courage`),
            // wired into the call sites that actually use it
            // (`CHARGE_MIN_COURAGE`, `OBSERVE_SWORDFIGHT` distance,
            // courage_distance, etc).
            let min = (expected_points as f32
                * parameters_ai::AI_MIN_LOOKFORHELPFLAG_SEEK_POINT_FACTOR)
                as u16;

            // The original game's rectangular random sampling returns
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
        let debug_selection = seek_area_selection_debug_matches(frame, creation_order);

        for &idx in near_sorted {
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
                crate::ai_enemy::parity_trace::SeekareaPhase4Candidate {
                    frame: &(frame),
                    owner_handle: &(self.base.me),
                    owner_creation_order: &(optional_u32(creation_order)),
                    candidate_ordinal: &(phase4_attempts),
                    point_id: &(global.seek_points[idx].id),
                    point_index: &(idx),
                    norm: &(square_norms[idx]),
                    norm_bits: &(square_norms[idx].to_bits()),
                    frame_when_full_interest: &(global.seek_points[idx].frame_when_full_interest),
                    interest: &(interest),
                    attempt_raw: &(optional_u32(attempt_raw)),
                    attempt_mod: &(attempt),
                    attempt_result: &(accepted),
                    insertion_raw: &(optional_u32(insertion_raw)),
                    insertion_index: &(optional_usize(insertion_index)),
                    accumulator_before_bits: &(accumulator_before_bits),
                    accumulator_after_bits: &(count_f.to_bits()),
                }
                .emit();
            }
        }

        if debug_selection {
            crate::ai_enemy::parity_trace::SeekareaSelectionSummary {
                frame: &(frame),
                owner_handle: &(self.base.me),
                owner_creation_order: &(creation_order),
                center_x: &(center.x),
                center_y: &(center.y),
                standard_radius: &(standard_radius),
                near_points: &(near_sorted.len()),
                expected_for_one: &(expected_points_for_one),
                visible_friends: &(seeking_friends),
                clears_help: &(clears_help),
                expected_before_help_random: &(expected_points_before_help_random),
                expected_points: &(expected_points),
                phase4_attempts: &(phase4_attempts),
                phase4_accepts: &(phase4_accepts),
                preselection_rng_draws: &(preselection_rng_draws),
                phase4_rng_draws: &(phase4_attempts + phase4_accepts),
                selection_rng_draws: &(preselection_rng_draws + phase4_attempts + phase4_accepts),
                accepted_interest_sum: &(count_f),
            }
            .emit();
            crate::ai_enemy::parity_trace::SeekareaSelectionExtra {
                frame: &(frame),
                owner_creation_order: &(creation_order),
                flags: &(flags.bits()),
                seek_direction: &(seek_direction),
                center_level: &(center.level),
                obligatory: &(obligatory_idx.map(|i| global.seek_points[i].id)),
                obligatory2: &(obligatory2_idx.map(|i| global.seek_points[i].id)),
                selected_random: &(selected_random
                    .iter()
                    .map(|&i| global.seek_points[i].id)
                    .collect::<Vec<_>>()),
            }
            .emit();
        }

        selected_random
    }

    pub(crate) fn append_personal_area_seek_points(
        &mut self,
        sim: &SimulationContext,
        spec: SeekAreaSpec,
        frame: u32,
        creation_order: Option<u32>,
    ) {
        let SeekAreaSpec {
            flags,
            seek_direction,
            ..
        } = spec;
        // ── Phase 6: personal seek points (postprocessing) ──

        let debug_phase6 = seek_area_phase6_debug_matches(frame, creation_order);
        if debug_phase6 {
            crate::ai_enemy::parity_trace::SeekAreaPhase6::Phase6Before {
                frame: frame,
                owner_handle: self.base.me,
                owner_creation_order: creation_order
                    .expect("phase6 diagnostic matched an owner without creation order"),
                state: self.base.current_state as u32,
                substate: self.base.current_substate as u32,
                flags: flags.bits(),
                seek_direction,
                list_size: self.my_seek_points.len(),
                list_empty: self.my_seek_points.is_empty(),
                location_first: flags.contains(SeekFlags::LOCATION_FIRST),
                location_end: flags.contains(SeekFlags::LOCATION_END),
                personal1_constructor: if !flags.contains(SeekFlags::LOCATION_FIRST) {
                    "none"
                } else if seek_direction == UNDEFINED_DIRECTION {
                    "position"
                } else {
                    "direction"
                },
            }
            .emit();
            crate::ai_enemy::parity_trace::SeekareaPhase6Center {
                frame: &(frame),
                owner_creation_order: &(creation_order),
                center_x: &(self.seek_center.x),
                center_y: &(self.seek_center.y),
                seek_position_x: &(self.base.seek_position.x),
                seek_position_y: &(self.base.seek_position.y),
            }
            .emit();
        }

        if flags.contains(SeekFlags::LOCATION_FIRST) {
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
                crate::ai_enemy::parity_trace::SeekAreaPhase6::Phase6Personal1 {
                    frame: frame,
                    owner_creation_order: creation_order
                        .expect("phase6 diagnostic matched an owner without creation order"),
                    constructor: if seek_direction == UNDEFINED_DIRECTION {
                        "position"
                    } else {
                        "direction"
                    },
                    inserted_id: 1111,
                    list_size: self.my_seek_points.len(),
                }
                .emit();
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
            crate::ai_enemy::parity_trace::SeekAreaPhase6::Phase6After {
                frame: frame,
                owner_creation_order: creation_order
                    .expect("phase6 diagnostic matched an owner without creation order"),
                personal2_inserted: insert_personal2,
                personal2_constructor: if insert_personal2 { "position" } else { "none" },
                list_size: self.my_seek_points.len(),
            }
            .emit();
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
    // Go to the next seek point or return to duty
    // -----------------------------------------------------------------------

    /// Advance to the next seek point, or return to duty if none remain.
    /// Checks interest and lock state, skipping uninteresting or locked
    /// points.
    pub(crate) fn seek_next_point(
        &mut self,
        _env: ThinkEnv<'_>,
        _global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<()> {
        Err(crate::ai::DutyCall {
            flags: DutyFlags::empty(),
            think_result: false,
            tail: crate::ai::DutyTail::SeekNextPoint,
            after: Vec::new(),
        })
    }

    /// Search for a missing PC.
    /// Begins a sweep of the checkpoint charly's patrol path:
    ///
    /// * Officers re-enter [`Substate::SeekingCharlyWatching`] and let
    ///   the existing missing-PC alert flow run.
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
    pub(crate) fn search_charly(
        &mut self,
        env: ThinkEnv<'_>,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<()> {
        let ThinkEnv { ctx, .. } = env;
        self.base.set_emoticon(EmoticonType::QuestionMark);

        // Officer arm.
        if self.get_rank() == ProfileRank::Officer {
            // The original game directly raises the missed-target alert while the officer is
            // still in DEFAULT_LOOKING_FOR_CHARLY.  That ordering is
            // observable: stopping actions for an area search preserves a macro
            // in the two checkpoint-look substates. Do not insert a synthetic
            // SEEKING_CHARLY_WATCHING/EventDone boundary here.
            self.missed_charly_alert(env, global)?;
            return Ok(());
        }

        // Soldier/knight prelude.
        self.base.say(Remark::MissesCharly);
        self.search_charly_way.clear();
        self.base.macro_in_progress = false;
        self.current_task_priority = task_priority::MISSED_FRIEND;
        self.seeking_charly = true;

        // No checkpoint → return to duty.
        if self.base.checkpoint_charly.is_none() {
            self.return_to_duty_default(env)?;
            return Ok(());
        }
        let Some(view) = ctx.entity_view(self.base.checkpoint_charly) else {
            self.return_to_duty_default(env)?;
            return Ok(());
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
                                // Waypoint positioning copies the authored
                                // sector pointer as well as x/y/level.  A
                                // missing sector makes an otherwise valid
                                // checkpoint-search movement fail before pathfinding.
                                sector: crate::position_interface::SectorHandle::new(w.sector),
                                level: w.level,
                            })
                            .collect();
                        // Nearest waypoint by maximum norm.
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
            self.return_to_duty_default(env)?;
            return Ok(());
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
        Ok(())
    }

    /// Report a failed checkpoint search and either delegate it or begin the
    /// area search locally. The original game's target search invokes this synchronously
    /// for officers; completion of watching a checkpoint member is its other caller.
    pub(super) fn missed_charly_alert(
        &mut self,
        env: ThinkEnv<'_>,
        global: &mut AiGlobalState,
    ) -> crate::ai::AiFlow<()> {
        let ThinkEnv { ctx, .. } = env;
        self.base.say(Remark::DidntFindCharly);
        let my_pos = ctx.position;
        self.base.seek_position = my_pos;
        self.base
            .my_reconnaissance_report
            .update(ReportType::MissedCharly, my_pos);
        self.base.my_reconnaissance_report.charly = self.base.checkpoint_charly;
        self.base.frame_when_enemy_detected = ctx.frame;
        if let Some(checkpoint_charly) = self.base.checkpoint_charly {
            self.base
                .outbox
                .actor
                .set_reported_to_officer
                .push((checkpoint_charly, false));
        }

        let alert_handled = match self.get_rank() {
            ProfileRank::Soldier => {
                self.alert_officer(crate::ai::OfficerAlertCaller::SeekMissedCharly {
                    center: my_pos,
                })?;
                true
            }
            ProfileRank::Officer => self.alert_soldiers(
                my_pos,
                SeekFlags::CHARLY_SEEK.bits(),
                env,
                AlertSoldiersFailureContinuation::SeekMissedCharly { center: my_pos },
            )?,
            ProfileRank::Knight | ProfileRank::None => false,
        };
        if alert_handled {
            return Ok(());
        }

        let charly_has_path = ctx
            .expect_entity_view(
                self.base.checkpoint_charly,
                "missed-charly checkpoint charly",
            )
            .has_patrol_path;
        let radius = if charly_has_path {
            parameters_ai::AI_PATROL_CHARLY_SEEK_RADIUS as u16
        } else {
            parameters_ai::AI_FIX_CHARLY_SEEK_RADIUS as u16
        };
        self.seek_area(
            env,
            my_pos,
            radius,
            SeekFlags::LOCATION_FIRST | SeekFlags::CHARLY_SEEK,
            UNDEFINED_DIRECTION,
            global,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
