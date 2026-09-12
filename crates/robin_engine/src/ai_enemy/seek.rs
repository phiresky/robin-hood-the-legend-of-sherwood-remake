//! Seek-and-search behaviours: the area-search seek-point loop, body
//! examination, search-for-charly, run-to-free-net-victim,
//! find-door-enemy-could-be-behind, dead-body-alert dispatch, and the
//! flee primitive.

use crate::ai::*;
use crate::parameters_ai;
use crate::position_interface::INVERSE_ASPECT_RATIO;

fn seek_area_selection_debug_matches(frame: u32, creation_order: Option<u32>) -> bool {
    use crate::engine::diagnostics::ParityGate;
    static GATE: std::sync::OnceLock<ParityGate<2>> = std::sync::OnceLock::new();
    let gate = GATE.get_or_init(|| {
        ParityGate::from_env(
            "PARITY_DEBUG_SEEK_AREA_OWNER_POSITION",
            [
                "PARITY_DEBUG_SEEK_AREA_FRAME",
                "PARITY_DEBUG_SEEK_AREA_CREATION_ORDER",
            ],
        )
    });
    gate.enabled() && gate.matches_required([Some(frame), creation_order])
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

#[inline]
fn legacy_seek_direction_delta(direction: u16, seek_direction: u16) -> u16 {
    direction.wrapping_add(16).wrapping_sub(seek_direction)
}

/// Reconstruct the sector identity carried by the original game's position when a
/// compact Rust stimulus retained only the point and layer.
///
/// In particular, `CALL_INSTRUCTION` copies an officer's seek point into each
/// group member. The original game copies the complete position; a sector-less
/// Rust copy would make the personal seek point fail movement checks before route
/// construction and recursively consume unrelated global seek points.
fn resolve_seek_area_center_sector(mut center: Position, ctx: &AiContext) -> Position {
    if center
        .sector
        .is_some_and(|sector| sector.arena_index().is_some())
    {
        return center;
    }

    let point = crate::coordinates::MapPoint::new(center.x, center.y);
    let reference = crate::coordinates::MapPoint::new(ctx.position.x, ctx.position.y);
    let hit = ctx.fast_grid.get_sector(point, reference, center.level);
    if let Some(exact_sector) = hit.sector_handle()
        && center
            .sector
            .is_none_or(|authored| authored == exact_sector)
    {
        // Tactic seek points and compact stimuli retain the public sector
        // number but not the original game's sector reference. Recover the exact arena
        // identity only when the spatial result agrees with that authored
        // number; a conflicting authored sector remains authoritative.
        center.sector = Some(exact_sector);
    }
    center
}

use super::util::{pos_distance, resolve_seek_point_id, resolve_seek_point_mut, vec_to_sector};
use super::{
    AlertSoldiersFailureContinuation, EnemyAi, ProfileRank, SeekFlags, UNDEFINED_DIRECTION,
    task_priority,
};

/// Immutable inputs shared by the candidate and personal-point phases.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct SeekAreaSpec {
    center: Position,
    standard_radius: u16,
    flags: SeekFlags,
    seek_direction: u16,
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
        let center = resolve_seek_area_center_sector(center, ctx);
        tracing::trace!(
            npc = self.base.me,
            state = ?self.base.current_state,
            substate = ?self.base.current_substate,
            center_x = center.x,
            center_y = center.y,
            standard_radius,
            ?flags,
            seek_direction,
            "starting area search"
        );
        self.base.stop_all();

        // Clear any prior stare-at-target focus so the
        // eye-tracking view cone doesn't stick on a stale primary
        // target while we sweep seek points. Drained by `engine/ai.rs`
        // → `unfocus`.
        self.base.outbox.actor.set_unfocus();

        // Royalists just return to duty.
        if ctx.is_player_aligned() {
            self.return_to_duty_default(sim, ctx, tick);
            return;
        }

        // Company 100 (combat trainer dummy) just returns to duty.
        if self.company_number == 100 {
            self.return_to_duty_default(sim, ctx, tick);
            return;
        }

        if !flags.contains(SeekFlags::CHARLY_SEEK) {
            // Clear the target checkpoint through the helper so
            // the `DETECTABLE_MISSED_FRIEND` list is cleared and
            // `sorrow_level` is zeroed alongside the field write.
            self.base.set_checkpoint_charly(None);
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

        self.rebuild_area_search_beggars(ctx);

        // Store seek flags and center
        self.seek_flags =
            flags | (flags & (SeekFlags::LOOK_FOR_HELP_AFTER | SeekFlags::REPORT_OFFICER_AFTER));
        self.seek_center = center;
        self.my_seek_points.clear();
        self.seek_point_view_directions.clear();

        let spec = SeekAreaSpec {
            center,
            standard_radius,
            flags,
            seek_direction,
        };

        // ── Build seek point list from global array ──
        // Gate on `standard_radius > 0 && !is_combat_trainer`. Combat
        // trainers fall through to the `LOCATION_FIRST/END`
        // assert/personal-seek-point branch.
        if standard_radius > 0 && !self.combat_trainer {
            self.append_global_area_seek_points(sim, spec, global, ctx, tick);
        } else {
            // standard_radius == 0: only personal seek points
            debug_assert!(
                flags.intersects(SeekFlags::LOCATION_FIRST | SeekFlags::LOCATION_END),
                "area search with radius 0 must have LOCATION_FIRST or LOCATION_END"
            );
        }

        self.append_personal_area_seek_points(sim, spec, global, ctx, tick);

        tracing::trace!(
            npc = self.base.me,
            frame = ctx.frame,
            seek_flags = ?self.seek_flags,
            list = ?self.my_seek_points,
            "area search built its seek point list"
        );

        // Clear actual seek point (critical — missing caused memory
        // bugs).
        self.actual_seek_point = None;

        assert!(
            !self.my_seek_points.is_empty(),
            "area search must produce at least one seek point"
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

    fn rebuild_area_search_beggars(&mut self, ctx: &AiContext) {
        // For sufficiently intelligent non-trainer soldiers, the original game
        // clears `DETECTABLE_BEGGAR` and immediately re-adds every actor for
        // who is a real or disguised beggar. This is authoritative list
        // state, not merely preparation for the next detection refresh: a
        // frame dump taken after area-search setup already contains the rebuilt list.
        if (self.get_iq(ctx) as i32) >= parameters_ai::CHECK_BEGGAR_MIN_IQ && !self.combat_trainer {
            use crate::element::{DetectableType, Posture};

            self.base
                .outbox
                .actor
                .delete_detectable_type(DetectableType::Beggar);
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
            self.base
                .outbox
                .actor
                .detectable_mutations
                .extend(beggars.into_iter().map(|(_, _, entity_id)| {
                    crate::ai::DetectableMutation::Add(entity_id, DetectableType::Beggar)
                }));
            self.beggar_to_examine = None;
        }
    }

    fn append_global_area_seek_points(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        spec: SeekAreaSpec,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        let candidates = SeekAreaCandidates::new(spec, global);
        let square_norms = &candidates.square_norms;
        let near_sorted = &candidates.near_sorted;
        let obligatory_idx = candidates.obligatory_idx;
        let center = spec.center;
        let current_frame = ctx.frame;
        if seek_area_selection_debug_matches(ctx.frame, ctx.original_creation_order) {
            for (i, sp) in global.seek_points.iter().enumerate() {
                eprintln!(
                    "SEEKAREA {{\"event\":\"point_dump\",\"frame\":{},\"index\":{},\"id\":{},\"x\":{},\"y\":{},\"level\":{},\"center\":[{},{},{}],\"norm\":{},\"norm_bits\":{},\"near\":{},\"frame_when_full_interest\":{}}}",
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
                    sp.frame_when_full_interest,
                );
            }
        }

        // If nearest point was recently examined, don't look for help
        if let Some(&first_idx) = near_sorted.first()
            && global.seek_points[first_idx].calculate_interest(current_frame) < 90
        {
            self.seek_flags &= !SeekFlags::LOOK_FOR_HELP_AFTER;
        }

        let selected_random =
            self.select_area_seek_points(sim, spec, &candidates, global, ctx, tick);
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
        sim: &crate::sim_rng::SimulationContext,
        spec: SeekAreaSpec,
        candidates: &SeekAreaCandidates,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
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
        let current_frame = ctx.frame;
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
        let debug_selection =
            seek_area_selection_debug_matches(ctx.frame, ctx.original_creation_order);

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

        selected_random
    }

    fn append_personal_area_seek_points(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        spec: SeekAreaSpec,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        let SeekAreaSpec {
            flags,
            seek_direction,
            ..
        } = spec;
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
            // Searching for a door updates seek_center in place.
            // Copy the field out, update it, then
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
    pub fn seek_next_point(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        let current_frame = ctx.frame;

        // Unlock the previous seek point
        // The original game unlocks the current seek point here but deliberately retains
        // the pointer until a new candidate is assigned below.  In
        // particular, a beggar detour returns with the old point identity;
        // when seeking resumes, the next entry unlocks that shared point a
        // second time.  Another soldier may have locked it during the detour.
        if let Some(prev_id) = self.actual_seek_point
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

        // Strip empty entries as protection against a corrupt list.
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
            debug_assert!(
                !self
                    .beggar_to_examine
                    .is_some_and(|handle| self.beggars_to_control.contains(&handle.get()))
            );
            self.beggar_to_examine = self.beggars_to_control.pop().map(AiEntityHandle::new);
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
            self.return_to_duty_default(sim, ctx, tick);

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
                "next seek point: seek list exhausted"
            );
            if quiet_report && !pending_followup {
                self.base.say(Remark::EndsSearch);
            }
            return;
        }

        // Pop the next seek point
        let next_id = self.my_seek_points.remove(0);
        // The original game assigns the current seek point before testing the candidate.
        // When a locked or uninteresting point recurses into next-point selection,
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

        // The original game short-circuits on a locked candidate, which is
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
        let seek_pos = resolve_seek_area_center_sector(seek_pos, ctx);

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
    // Find a door the enemy could be behind
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

            // The original game compares the exact sector reference carried by position,
            // not the public sector number. Duplicate public numbers occur
            // in real levels; accepting a door from the wrong arena changes
            // the personal seek point to that door's inside position and can
            // immediately recurse through EVENT_COULDNT_REACHPOINT.
            let Some(center_sector) = seek_center.sector else {
                continue;
            };
            if door_info.sector_out != u16::from(center_sector) {
                continue;
            }
            if let Some(center_index) = center_sector.arena_index() {
                let door_index = door_info.sector_out_index.unwrap_or_else(|| {
                    panic!(
                        "building door {} exterior sector {} lacks exact arena identity required by seek center {center_index:?}",
                        door_info.door_index, door_info.sector_out
                    )
                });
                if door_index != center_index {
                    continue;
                }
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
            let diff = legacy_seek_direction_delta(door_dir, seek_direction) & 15;
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
    // Corpse discovery triggers a rank-dispatched alert
    // Corpse-discovery alert flow.
    // -----------------------------------------------------------------------

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
                    && self.base.antagonist.is_none()
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
                        // Officer alerting requests an approach synchronously, and the original game
                        // inspects the unreachable-point flag before corpse-alert processing
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

    /// Resume soldier corpse-alert processing after its call to
    /// officer alerting. A failed approach is consumed synchronously and falls back to the
    /// corpse search; a successful route has no further tail.
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
        // This fallback is the statement immediately following
        // the officer alert's synchronous approach inside the original enclosing
        // Think. Rust resumes it from owner work after releasing the AI
        // borrow, so `think_recursion_depth` alone no longer records that
        // ownership. Any movement selected by area search must still deliver a
        // synchronous route failure to that open logical Think, allowing
        // next-point selection to try the following candidate in the same frame.
        if !self.base.outbox.actor.orders.is_empty() {
            self.base.completion_latch_inside_think = true;
        }
    }
    // -----------------------------------------------------------------------
    // Body examination
    // -----------------------------------------------------------------------

    /// Run to the nearest net covering a stuck victim and prepare to
    /// remove it.
    ///
    /// Picks the covering net with minimum maximum-norm distance from self,
    /// records the chosen net in `interesting_object` (so
    /// `SeekingTakingNet` drives the SEARCH+TAKE sequence against the
    /// right net), and routes either to the net (reachable) or to the
    /// victim (emergency fallback) depending on whether straight movement
    /// is allowed.
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

        // Collect nets covering the victim.
        // (reverse-index the net → victims map) then pick the minimum-
        // maximum-norm-distance net. The reverse index lives on the view as
        // `covering_nets`, pre-scanned by `build_entity_views`.
        // Maximum-norm distance stretches Y by `INVERSE_ASPECT_RATIO` before
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
            // a net was found. Reaching here means
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
        self.base.detected_body = Some(AiEntityHandle::new(victim));
        self.base.interesting_object = Some(AiEntityHandle::new(net.handle));

        // If the victim → net segment is clear on the
        // victim's layer for my move-box, walk up to the net and stop
        // at the radius plus 15. Otherwise fall back to the victim's
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

        // Enter the seeking-net state, approach at a run, and launch a
        // 10-tick timer. `go_near` folds the state change in.
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
    pub fn search_charly(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        self.base.set_emoticon(EmoticonType::QuestionMark);

        // Officer arm.
        if self.get_rank() == ProfileRank::Officer {
            // The original game directly raises the missed-target alert while the officer is
            // still in DEFAULT_LOOKING_FOR_CHARLY.  That ordering is
            // observable: stopping actions for an area search preserves a macro
            // in the two checkpoint-look substates. Do not insert a synthetic
            // SEEKING_CHARLY_WATCHING/EventDone boundary here.
            self.missed_charly_alert(sim, global, ctx, tick, grid);
            return;
        }

        // Soldier/knight prelude.
        self.base.say(Remark::MissesCharly);
        self.search_charly_way.clear();
        self.base.macro_in_progress = false;
        self.current_task_priority = task_priority::MISSED_FRIEND;
        self.seeking_charly = true;

        // No checkpoint → return to duty.
        if self.base.checkpoint_charly.is_none() {
            self.return_to_duty_default(sim, ctx, tick);
            return;
        }
        let Some(view) = ctx.entity_view(self.base.checkpoint_charly) else {
            self.return_to_duty_default(sim, ctx, tick);
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
            self.return_to_duty_default(sim, ctx, tick);
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

    /// Report a failed checkpoint search and either delegate it or begin the
    /// area search locally. The original game's target search invokes this synchronously
    /// for officers; completion of watching a checkpoint member is its other caller.
    pub(super) fn missed_charly_alert(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
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
                self.alert_officer(sim, my_pos, SeekFlags::CHARLY_SEEK.bits(), ctx, tick)
            }
            ProfileRank::Officer => self.alert_soldiers(
                my_pos,
                SeekFlags::CHARLY_SEEK.bits(),
                global,
                grid,
                ctx,
                tick,
                AlertSoldiersFailureContinuation::SeekMissedCharly { center: my_pos },
            ),
            ProfileRank::Knight | ProfileRank::None => false,
        };
        if alert_handled {
            return;
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
            sim,
            my_pos,
            radius,
            SeekFlags::LOCATION_FIRST | SeekFlags::CHARLY_SEEK,
            UNDEFINED_DIRECTION,
            global,
            ctx,
            tick,
        );
    }

    pub fn run_to_examine_body(
        &mut self,
        body: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
    ) {
        // Body examination: if stuck under a net, delegate to net-victim
        // rescue; otherwise focus, mark X
        // emoticon, and run up to the body.
        let view = ctx.entity_view(body);
        let stuck = view.map(|v| v.stuck_under_net).unwrap_or(false);
        if stuck {
            // Run to free the net victim.
            self.run_to_free_net_victim(body, ctx, grid);
            return;
        }

        self.base.detected_body = Some(AiEntityHandle::new(body));
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
        // Set the X-mark emoticon.
        self.base.set_emoticon(EmoticonType::XMark);
        // Enter the seeking-body state.
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
        // Prune from the front while the first body has recovered or woken up.
        while let Some(&first) = self.other_bodies_to_examine.first() {
            // Body queues deliberately contain out-of-order humans. The
            // tactical nearby-fighter list filters those actors out before AI
            // dispatch, so absence there does not mean the body recovered.
            // Use the complete handle-indexed entity snapshot to check
            // whether each body has recovered, as in the original game.
            // Combat readiness is a
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
mod tests;
