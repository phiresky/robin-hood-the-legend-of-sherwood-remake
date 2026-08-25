//! Swordfight evaluation, parade decisions, propose/launch strikes.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;
use crate::combat::{self};
use crate::element::{ActionState, Command, Entity, EntityId};
use crate::profiles::WeaponThrustKind;
use crate::weapons::SwordStrike;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SwordfightDistanceAdjustment {
    None,
    Closer,
    Farther,
}

pub(super) fn swordfight_distance_adjustment(
    distance: f32,
    my_minimum: f32,
    my_maximum: f32,
    opponent_maximum: f32,
    last_motion_was_step_back: bool,
) -> SwordfightDistanceAdjustment {
    if distance > my_maximum && distance > opponent_maximum && !last_motion_was_step_back {
        SwordfightDistanceAdjustment::Closer
    } else if distance < my_minimum {
        SwordfightDistanceAdjustment::Farther
    } else {
        SwordfightDistanceAdjustment::None
    }
}

fn is_within_smalltalk_strike_range(maximal_range: u16, squared_distance: f32) -> bool {
    let maximal_range = u32::from(maximal_range);
    // Original assigns the UWORD range to ULONG, squares it, and casts the
    // floating-point SquareNorm to ULONG before comparing.
    maximal_range * maximal_range >= squared_distance as u32
}

#[derive(Clone, Copy)]
struct ReactiveStepBackDebug {
    frame: u32,
    creation_order: u32,
}

fn reactive_step_back_debug_config() -> Option<ReactiveStepBackDebug> {
    std::env::var_os("PARITY_DEBUG_REACTIVE_STEP_BACK")?;
    let parse_required = |name: &str| {
        let value = std::env::var(name)
            .unwrap_or_else(|_| panic!("{name} is required for reactive step-back diagnostic"));
        value.parse::<u32>().unwrap_or_else(|error| {
            panic!("invalid {name}={value:?} for reactive step-back diagnostic: {error}")
        })
    };
    Some(ReactiveStepBackDebug {
        frame: parse_required("PARITY_DEBUG_REACTIVE_STEP_BACK_FRAME"),
        creation_order: parse_required("PARITY_DEBUG_REACTIVE_STEP_BACK_CREATION_ORDER"),
    })
}

pub(super) fn reactive_sword_debug_frame_matches(frame: u32) -> bool {
    if std::env::var_os("PARITY_DEBUG_REACTIVE_SWORD").is_none() {
        return false;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for reactive sword diagnostic: {error}")
            })
        })
    };
    parse_filter("PARITY_DEBUG_REACTIVE_SWORD_FRAME").is_none_or(|value| value == frame)
}

pub(super) fn reactive_sword_debug_creation_order_matches(creation_order: u32) -> bool {
    std::env::var("PARITY_DEBUG_REACTIVE_SWORD_CREATION_ORDER")
        .ok()
        .is_none_or(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!(
                    "invalid PARITY_DEBUG_REACTIVE_SWORD_CREATION_ORDER={value:?} for reactive sword diagnostic: {error}"
                )
            }) == creation_order
        })
}

pub(super) fn opponent_sword_strike_time_limit(
    animation: Option<crate::order::OrderType>,
    action_done_timing: impl FnOnce() -> crate::sprite::ActionDoneTiming,
) -> i16 {
    use crate::order::OrderType as OT;

    if !matches!(
        animation,
        Some(
            OT::StrikingStraightSword
                | OT::StrikingStraightStrongSword
                | OT::ExecutingSword
                | OT::StrikingRightSword
                | OT::StrikingLeftSword
                | OT::StrikingRoundRightSword
                | OT::StrikingRoundLeftSword
                | OT::StrikingSemiroundRightSword
                | OT::StrikingSemiroundLeftSword
        )
    ) {
        return 1000;
    }

    match action_done_timing() {
        crate::sprite::ActionDoneTiming::Frames(-1) => 1000,
        crate::sprite::ActionDoneTiming::Frames(frames) => frames,
        // InitializeActionDone calls this state "impossible". The legacy
        // vector over-read observed at S075 frame 731 produced -16230; use a
        // defined deadline below every non-negative strike/parry startup
        // rather than fabricating a permissive `-1` result.
        crate::sprite::ActionDoneTiming::Impossible => i16::MIN,
    }
}

fn resolve_stale_impossible_action_done(
    captured_deadline: Option<i16>,
) -> crate::sprite::ActionDoneTiming {
    captured_deadline.map_or(
        crate::sprite::ActionDoneTiming::Impossible,
        crate::sprite::ActionDoneTiming::Frames,
    )
}

impl EngineInner {
    /// Return the proposal deadline using Original's deliberately mixed view:
    /// `RHElementActor::GetAnimation()` exposes the live selected order, while
    /// `RHSprite::GetFramesFromNowTillActionDone()` still reads the sprite's
    /// current row even when that order has not executed yet
    /// (`RHelementactorhuman.cpp:13010-13022`).
    ///
    /// Keep that stale-row boundary -- an ordinary `-1` means the prior
    /// action point has passed and remains the permissive 1000 fallback.  An
    /// impossible marker is different even when the selected order is newer
    /// than the sprite row: Original still walks the stale row's delay vector
    /// to `0xffff` through unchecked `std::vector::operator[]`, wrapping the
    /// running `SWORD`. That allocator residue is not reproducible from game
    /// state, so parity replays may provide the captured wrapped result.
    pub(super) fn opponent_sword_strike_time_limit_for_actor(
        &mut self,
        proposer: EntityId,
        opponent: EntityId,
    ) -> Option<i16> {
        let (animation, timing, stale_impossible) = {
            let animation = self.live_actor_animation(opponent)?;
            let entity = self.get_entity(opponent)?;
            let actor = entity.actor_data()?;
            let sprite = &entity.element_data().sprite;
            let timing = sprite.action_done_timing();
            let stale_impossible = timing == crate::sprite::ActionDoneTiming::Impossible
                && actor
                    .installed_order
                    .is_some_and(|order| order.order_id.get() != sprite.last_processed_order_id);
            (animation, timing, stale_impossible)
        };
        let timing = if stale_impossible {
            let key = (
                self.world.original_creation_order(proposer),
                self.world.original_creation_order(opponent),
            );
            resolve_stale_impossible_action_done(
                self.control
                    .original_impossible_action_done_deadlines
                    .get_mut(&key)
                    .and_then(std::collections::VecDeque::pop_front),
            )
        } else {
            timing
        };
        Some(opponent_sword_strike_time_limit(Some(animation), || timing))
    }
}

fn required_hth_weapon_profile<'a>(
    entity: &Entity,
    entity_id: EntityId,
    profiles: &'a crate::profiles::ProfileManager,
    context: &str,
) -> &'a crate::profiles::HtHWeaponProfile {
    let weapon_id =
        match entity {
            Entity::Pc(pc) => {
                profiles
                    .get_character(pc.pc.profile_index)
                    .unwrap_or_else(|| {
                        panic!(
                            "{context}: PC {entity_id:?} references missing character profile {:?}",
                            pc.pc.profile_index
                        )
                    })
                    .hth_weapon_id
            }
            Entity::Soldier(soldier) => profiles
                .get_soldier(soldier.soldier.soldier_profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "{context}: soldier {entity_id:?} references missing soldier profile {:?}",
                        soldier.soldier.soldier_profile_index
                    )
                })
                .hth_weapon_id,
            other => panic!(
                "{context}: combatant {entity_id:?} has no hand-to-hand profile ({:?})",
                std::mem::discriminant(other)
            ),
        };
    profiles.get_hth_weapon(weapon_id).unwrap_or_else(|| {
        panic!(
            "{context}: combatant {entity_id:?} references missing hand-to-hand weapon {weapon_id}"
        )
    })
}

impl EngineInner {
    // ─── Smalltalk initiative ─────────────────────────────────────

    /// Give smalltalk initiative to `entity`, taking it from their
    /// principal opponent if they are mutual principal opponents.
    ///
    /// Sets both `smalltalk_initiative` and
    /// `received_smalltalk_initiative` so the next smalltalk pass
    /// consumes the received flag once and skips the
    /// loss-of-initiative check.
    pub(super) fn take_smalltalk_initiative(&mut self, entity_id: EntityId) {
        let principal = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied());

        let Some(principal_id) = principal else {
            return;
        };

        // Original `IsSwordfighting()` is exactly
        // `mlistOpponents.Size() != 0`; it does not inspect the actor action
        // state. This matters while EnterSwordfight installs the reciprocal
        // relationship before either actor has finished raising their sword.
        let self_swordfighting = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .map(|human| !human.opponents.is_empty())
            .unwrap_or(false);
        let opp_swordfighting = self
            .get_entity(principal_id)
            .and_then(|e| e.human_data())
            .map(|human| !human.opponents.is_empty())
            .unwrap_or(false);
        if !(self_swordfighting && opp_swordfighting) {
            return;
        }

        if let Some(entity) = self.world.entities.get_mut(entity_id)
            && let Some(human) = entity.human_data_mut()
        {
            human.smalltalk_initiative = true;
            human.received_smalltalk_initiative = true;
        }

        // If mutual principal opponents, opponent loses initiative
        let is_mutual = self
            .get_entity(principal_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied())
            .map(|opp| opp == entity_id)
            .unwrap_or(false);

        if is_mutual
            && let Some(entity) = self.world.entities.get_mut(principal_id)
            && let Some(human) = entity.human_data_mut()
        {
            human.smalltalk_initiative = false;
        }
    }

    /// Recompute `relative_fighting_ability` for a single entity against
    /// the sum of its current opponents' fighting abilities.
    ///
    /// Returns 50 when both sides match (or when one side is
    /// missing); otherwise `100 * own / (own + opponents)`.
    ///
    /// Called whenever the opponent list changes (enter/quit swordfight,
    /// opponent purges).
    pub(super) fn recompute_relative_fighting_ability(
        &mut self,
        entity_id: EntityId,
        assets: &LevelAssets,
    ) {
        let opponents: Vec<EntityId> = match self.get_entity(entity_id).and_then(|e| e.human_data())
        {
            Some(h) => h.opponents.ids(),
            None => return,
        };

        let own_ability = self
            .get_entity(entity_id)
            .map(|e| {
                fighting_ability_from_profile(
                    e,
                    &assets.profile_manager,
                    self.control.sim_config.difficulty,
                )
            })
            .unwrap_or(50);

        let opponents_total: u16 = opponents
            .iter()
            .filter_map(|id| self.get_entity(*id))
            .map(|e| {
                fighting_ability_from_profile(
                    e,
                    &assets.profile_manager,
                    self.control.sim_config.difficulty,
                )
            })
            .fold(0u16, |acc, fa| acc.saturating_add(fa));

        let rfa = combat::compute_relative_fighting_ability(own_ability, opponents_total);

        if let Some(entity) = self.world.entities.get_mut(entity_id)
            && let Some(human) = entity.human_data_mut()
        {
            human.relative_fighting_ability = rfa;
        }
    }

    /// Maintains the spacing between two combatants in a smalltalk
    /// swordfight: when out of MAXIMAL range and we didn't just step
    /// back, take a step closer; when inside MINIMAL range, take a
    /// force-movement step back (with `find_authorized_position`
    /// fallback when the back-step is blocked).  Returns `true` when
    /// a MOVE element was launched (caller short-circuits the rest
    /// of swordfight evaluation).
    ///
    /// Guards:
    /// - Selected PC — player drives motion.
    /// - Combat-trainer soldier — stays put (training stance).
    ///
    /// Table-mode branch: when the principal opponent is paired by a
    /// jump line, delegate to `find_position_for_table_swordfight`
    /// and only emit a MOVE when the proposed slot is more than 1
    /// unit away (MaxNorm) and straight-reachable.
    ///
    /// Distance branch:
    /// - Compute stretch-Y 3D distance via
    ///   `INVERSE_SWORDFIGHT_ASPECT_RATIO`.
    /// - If `> max(MAXIMAL_self, MAXIMAL_opp)` and not just stepped
    ///   back → walk closer.
    /// - If `< MINIMAL_self` → walk away with force-movement.
    /// - Compute destination by normalising the 2D vector to opponent
    ///   and scaling by `geo_movement` (`-` for back-step).
    /// - Table-mode forward-only guard: when in different sectors,
    ///   reject backward motion.
    /// - When force-movement is set and the straight path is blocked,
    ///   try `find_authorized_position` to slide the destination into
    ///   a reachable slot.
    pub(super) fn update_swordfight_distance(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) -> bool {
        // Read all the geometry / profile data we need without holding
        // a borrow into self.world.entities.
        let snapshot = {
            let entity = self.get_entity(entity_id).unwrap_or_else(|| {
                panic!("EvaluateSwordfight distance owner {entity_id:?} is missing")
            });
            let human = entity.human_data().unwrap_or_else(|| {
                panic!("EvaluateSwordfight distance owner {entity_id:?} is not human")
            });
            let principal = human.opponents.first().copied().unwrap_or_else(|| {
                panic!("EvaluateSwordfight distance owner {entity_id:?} has no principal")
            });
            let last_step_back = human.last_motion_was_step_back_in_combat;
            let opp_jump_line = human.opponents.jump_line(0);

            // Selected PCs never reposition themselves during swordfighting.
            // Controlled soldiers use stance policy instead: Hold and
            // Defensive remain at their assigned position, while Aggressive
            // retains the original soldier AI's pursuit/repositioning.
            if entity.is_pc() && self.selected_pc_ids().contains(&entity_id)
                || entity.is_soldier() && !self.allied_allows_combat_movement(entity_id)
            {
                return false;
            }
            // Combat trainer stays put.
            let combat_trainer = entity.enemy_ai().map(|a| a.combat_trainer).unwrap_or(false);
            if combat_trainer {
                return false;
            }

            let pos_3d = entity.element_data().position();
            let pos_map = entity.element_data().position_map();
            let layer = entity.element_data().layer();
            let move_box = *entity.position_iface().get_move_box();
            let sector = entity
                .element_data()
                .sector()
                .map(i16::from)
                .unwrap_or_else(|| {
                    panic!("EvaluateSwordfight distance owner {entity_id:?} has no sector")
                });
            let backward_dist = (entity
                .element_data()
                .sprite
                .distance_for_animation(crate::order::OrderType::WalkingBackwardsSword)
                as f32)
                .abs();
            let weapon = required_hth_weapon_profile(
                entity,
                entity_id,
                &assets.profile_manager,
                "EvaluateSwordfight distance range",
            );
            let my_max_range =
                weapon.distance[crate::weapons::WeaponDistance::Maximal as usize] as f32;
            let my_min_range =
                weapon.distance[crate::weapons::WeaponDistance::Minimal as usize] as f32;
            (
                principal,
                last_step_back,
                opp_jump_line,
                pos_3d,
                pos_map,
                layer,
                move_box,
                sector,
                backward_dist,
                my_max_range,
                my_min_range,
            )
        };
        let (
            principal_id,
            last_step_back,
            opp_jump_line,
            my_pos_3d,
            my_pos_map,
            my_layer,
            my_move_box,
            my_sector,
            backward_dist,
            my_max_range,
            my_min_range,
        ) = snapshot;

        // ── Table-swordfight branch ───────────────────────────────
        if let Some(jl_idx) = opp_jump_line {
            let jump_line = self
                .world
                .fast_grid
                .level
                .jump_lines
                .get(u32::from(jl_idx) as usize)
                .cloned()
                .unwrap_or_else(|| {
                    panic!(
                        "EvaluateSwordfight distance owner {entity_id:?} references missing jump line {jl_idx:?}"
                    )
                });
            let dest = match find_position_for_table_swordfight(
                &self.world.entities,
                my_pos_map,
                my_sector,
                entity_id,
                principal_id,
                &jump_line,
            ) {
                Some(p) => p,
                None => return false,
            };
            // 1-unit MaxNorm dead-zone — skip if barely displaced.
            let dx = dest.x - my_pos_map.x;
            let dy = dest.y - my_pos_map.y;
            if dx.abs().max(dy.abs()) <= 1.0 {
                return false;
            }
            // Must be straight-reachable.
            if !self.world.fast_grid.is_straight_movement_authorized(
                my_pos_map,
                dest,
                my_layer,
                &my_move_box,
            ) {
                return false;
            }
            self.launch_swordfight_distance_move(
                entity_id,
                crate::coordinates::MapPoint {
                    x: dest.x,
                    y: dest.y,
                },
                my_layer,
            );
            return true;
        }

        // ── Distance branch ───────────────────────────────────────
        let opp = self.get_entity(principal_id).unwrap_or_else(|| {
            panic!(
                "EvaluateSwordfight distance owner {entity_id:?} references missing principal {principal_id:?}"
            )
        });
        let opp_pos_3d = opp.element_data().position();
        let opp_pos_map = opp.element_data().position_map();
        let opp_sector = opp
            .element_data()
            .sector()
            .map(i16::from)
            .unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight distance principal {principal_id:?} for {entity_id:?} has no sector"
                )
            });
        let opp_max_range = required_hth_weapon_profile(
            opp,
            principal_id,
            &assets.profile_manager,
            "EvaluateSwordfight principal distance range",
        )
        .distance[crate::weapons::WeaponDistance::Maximal as usize]
            as f32;

        // Stretch-Y 3D distance.  The stretching collapses the
        // isometric Y compression so distance comparisons line up
        // with the per-sword horizontal range constants.
        let dx = opp_pos_3d.x - my_pos_3d.x;
        let dy = (opp_pos_3d.y - my_pos_3d.y) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
        let dz = opp_pos_3d.z - my_pos_3d.z;
        let geo_distance = (dx * dx + dy * dy + dz * dz).sqrt();

        let mut geo_movement: f32 = 0.0;
        let mut force_movement = false;

        // Out-of-range → walk closer (unless we just stepped back,
        // in which case stay).
        match swordfight_distance_adjustment(
            geo_distance,
            my_min_range,
            my_max_range,
            opp_max_range,
            last_step_back,
        ) {
            SwordfightDistanceAdjustment::Closer => geo_movement = backward_dist,
            // Too close → walk back with force-movement.
            SwordfightDistanceAdjustment::Farther => {
                geo_movement = -backward_dist;
                force_movement = true;
            }
            SwordfightDistanceAdjustment::None => {}
        }

        if geo_movement == 0.0 {
            return false;
        }

        // Table-mode forward-only guard.  When we're in a different
        // sector to our opponent, only forward movement is permitted.
        if my_sector != opp_sector && geo_movement <= 0.0 {
            return false;
        }

        // Build the destination: normalise the 2D vector to opponent
        // and scale by `geo_movement`.
        let dist_map_dx = opp_pos_map.x - my_pos_map.x;
        let dist_map_dy = opp_pos_map.y - my_pos_map.y;
        let (move_dx, move_dy) = if geo_distance > f32::EPSILON {
            let scale = geo_movement / geo_distance;
            (dist_map_dx * scale, dist_map_dy * scale)
        } else {
            // Degenerate: pick a random direction.
            let sector = crate::sim_rng::u16(
                sim,
                crate::sim_rng::RngSite::MeleeDegenerateDirection,
                0..16,
            ) as i16;
            let (dx_s, dy_s) = crate::element::direction_vector_16(sector);
            (dx_s * geo_movement, dy_s * geo_movement)
        };

        let mut destination = crate::coordinates::MapPoint {
            x: my_pos_map.x + move_dx,
            y: my_pos_map.y + move_dy,
        };

        // Reachability test.
        let mut is_reachable = self.world.fast_grid.is_straight_movement_authorized(
            my_pos_map,
            destination,
            my_layer,
            &my_move_box,
        );

        // Force-movement fallback: try to slide the destination into
        // a reachable slot via `find_authorized_position_toward`.
        if force_movement && !is_reachable {
            let mut box_at_dest = my_move_box.translated(destination);
            if self.world.fast_grid.find_authorized_position_toward(
                &mut box_at_dest,
                my_pos_map,
                my_layer,
            ) && let Some(rect) = box_at_dest.to_geo().0
            {
                let center = rect.center();
                destination = crate::coordinates::MapPoint {
                    x: center.x,
                    y: center.y,
                };
                is_reachable = true;
            }
        }

        if !is_reachable {
            return false;
        }

        self.launch_swordfight_distance_move(
            entity_id,
            crate::coordinates::MapPoint {
                x: destination.x,
                y: destination.y,
            },
            my_layer,
        );
        true
    }

    /// Helper: launch a `Command::Move` sequence element with
    /// `WalkingUpright` action style for the swordfight distance
    /// adjustment.
    pub(super) fn launch_swordfight_distance_move(
        &mut self,
        actor_id: EntityId,
        destination: crate::coordinates::MapPoint,
        layer: u16,
    ) {
        let mut elem = crate::sequence::SequenceElement::new_movement(
            1,
            crate::element::Command::Move,
            Some(actor_id),
            crate::order::OrderType::WalkingUpright,
        );
        elem.data = crate::sequence::SequenceElementData::Movement {
            destination,
            layer,
            sector: None,
            gate_id: None,
            line_id: None,
            element: None,
            flags: crate::sequence::MoveFlags::empty(),
            tolerance: 0.0,
            direction: 0,
            action: crate::order::OrderType::WalkingUpright,
            speed_factor: 1.0,
            post_seek_sequence: None,
        };
        self.register_owned_element_deferred(elem);
    }

    /// Run the non-animation work in one human actor's WaitingSword Execute
    /// arm. The caller has already performed Turn/PerformAction for this exact
    /// owner slot.
    pub(crate) fn tick_waiting_sword_execute_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let entity = self
            .get_entity(entity_id)
            .unwrap_or_else(|| panic!("WaitingSword Execute owner {entity_id:?} is missing"));
        let human = entity
            .human_data()
            .unwrap_or_else(|| panic!("WaitingSword Execute owner {entity_id:?} is not human"));
        assert!(
            !entity.is_dead() && !human.unconscious,
            "WaitingSword Execute owner {entity_id:?} is dead or unconscious"
        );
        if let Entity::Soldier(soldier) = entity
            && soldier.is_soldier_observing_swordfight()
        {
            return;
        }

        // EvaluateSmalltalkHint precedes EvaluateSwordfight and suppresses the
        // latter whenever it launches a useful parry.
        if self.evaluate_smalltalk_hint(entity_id) {
            return;
        }
        self.trace_waiting_sword_evaluate_entry(entity_id);
        self.evaluate_swordfight_for(sim, assets, entity_id);
    }

    /// Emit the authoritative sequence selection at the exact boundary where
    /// WaitingSword enters `EvaluateSwordfight`.
    ///
    /// The dedicated tracing target keeps this inert unless explicitly
    /// enabled, for example with
    /// `RUST_LOG=parity_waiting_sword=debug`. It is intentionally diagnostic
    /// only: no replay schema or simulation state depends on these fields.
    fn trace_waiting_sword_evaluate_entry(&self, entity_id: EntityId) {
        if !tracing::enabled!(target: "parity_waiting_sword", tracing::Level::DEBUG) {
            return;
        }

        let current = self
            .orders
            .sequence_manager
            .current_order_for_actor(entity_id)
            .map(|(seq_id, elem_idx, order)| {
                let element = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .unwrap_or_else(|| {
                        panic!(
                            "WaitingSword parity diagnostic selected missing element ({seq_id:?}, {elem_idx})"
                        )
                    });
                (
                    seq_id,
                    elem_idx,
                    element.command,
                    order.order_id,
                    order.order_type,
                )
            });

        match current {
            Some((seq_id, elem_idx, command, order_id, order_type)) => {
                tracing::debug!(
                    target: "parity_waiting_sword",
                    frame = self.control.frame_counter,
                    owner = ?entity_id,
                    sequence_id = ?seq_id,
                    element_index = elem_idx,
                    command = ?command,
                    order_id = order_id.get(),
                    order_type = ?order_type,
                    "WaitingSword EvaluateSwordfight entry"
                );
            }
            None => {
                tracing::debug!(
                    target: "parity_waiting_sword",
                    frame = self.control.frame_counter,
                    owner = ?entity_id,
                    "WaitingSword EvaluateSwordfight entry has no current order"
                );
            }
        }
    }

    /// Owner-local translation of RHElementActorHuman::EvaluateSwordfight.
    fn evaluate_swordfight_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let opponents = self
            .get_entity(entity_id)
            .unwrap_or_else(|| panic!("EvaluateSwordfight owner {entity_id:?} is missing"))
            .human_data()
            .unwrap_or_else(|| panic!("EvaluateSwordfight owner {entity_id:?} is not human"))
            .opponents
            .clone();
        for opponent_id in opponents.iter().copied() {
            let opponent = self.get_entity(opponent_id).unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight owner {entity_id:?} references missing opponent {opponent_id:?}"
                )
            });
            assert!(
                opponent.human_data().is_some(),
                "EvaluateSwordfight owner {entity_id:?} opponent {opponent_id:?} is not human"
            );
        }

        if opponents.is_empty() {
            self.launch_element(crate::sequence::SequenceElement::new(
                1,
                Command::QuitSwordfight,
                Some(entity_id),
            ));
            return;
        }

        let first_principal = opponents[0];
        let principal_is_swordfighting = !self
            .get_entity(first_principal)
            .unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight owner {entity_id:?} principal {first_principal:?} vanished"
                )
            })
            .human_data()
            .unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight owner {entity_id:?} principal {first_principal:?} is not human"
                )
            })
            .opponents
            .is_empty();
        if !principal_is_swordfighting {
            return;
        }

        let (self_pos, self_sector, self_uber, tiredness, is_pc, num_opponents) = {
            let entity = self.get_entity(entity_id).unwrap_or_else(|| {
                panic!("EvaluateSwordfight owner {entity_id:?} vanished before snapshot")
            });
            let human = entity.human_data().unwrap_or_else(|| {
                panic!("EvaluateSwordfight owner {entity_id:?} lost human data")
            });
            let uber = required_hth_weapon_profile(
                entity,
                entity_id,
                &assets.profile_manager,
                "EvaluateSwordfight Uber range",
            )
            .distance[crate::weapons::WeaponDistance::Uber as usize] as f32;
            (
                entity.element_data().position(),
                entity
                    .element_data()
                    .sector()
                    .map(i16::from)
                    .unwrap_or_else(|| {
                        panic!("EvaluateSwordfight owner {entity_id:?} has no sector")
                    }),
                uber,
                human.tiredness,
                entity.is_pc(),
                human.opponents.len(),
            )
        };
        let (principal_pos, principal_sector, principal_uber) = {
            let principal = self.get_entity(first_principal).unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight owner {entity_id:?} principal {first_principal:?} vanished before range check"
                )
            });
            let uber = required_hth_weapon_profile(
                principal,
                first_principal,
                &assets.profile_manager,
                "EvaluateSwordfight principal Uber range",
            )
            .distance[crate::weapons::WeaponDistance::Uber as usize] as f32;
            (
                principal.element_data().position(),
                principal
                    .element_data()
                    .sector()
                    .map(i16::from)
                    .unwrap_or_else(|| {
                        panic!(
                            "EvaluateSwordfight principal {first_principal:?} for owner {entity_id:?} has no sector"
                        )
                    }),
                uber,
            )
        };

        let elevation_prune = (self_pos.z - principal_pos.z).abs() > MAX_ELEVATION_SWORDFIGHT
            && self_sector != principal_sector;
        // Original EvaluateSwordfight tears down the relationship and returns
        // immediately when the fighters are on incompatible elevations.  In
        // particular, it never reaches the later visibility query.
        if elevation_prune {
            self.delete_opponent(sim, assets, entity_id, first_principal);
            self.delete_opponent(sim, assets, first_principal, entity_id);
            self.evaluate_opponents(sim, assets, entity_id);
            self.evaluate_opponents(sim, assets, first_principal);
            return;
        }

        let dx = self_pos.x - principal_pos.x;
        let dy = self_pos.y - principal_pos.y;
        let dz = self_pos.z - principal_pos.z;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();
        let mut range_or_los_prune = distance > self_uber || distance > principal_uber;
        if !range_or_los_prune {
            let self_eye = self
                .get_entity(entity_id)
                .and_then(|entity| {
                    entity.compute_eyes_point(Some(crate::element::Posture::Upright))
                })
                .unwrap_or_else(|| {
                    panic!("EvaluateSwordfight owner {entity_id:?} has no upright eye point")
                });
            let principal_eye = self
                .get_entity(first_principal)
                .and_then(|entity| {
                    entity.compute_eyes_point(Some(crate::element::Posture::Upright))
                })
                .unwrap_or_else(|| {
                    panic!(
                        "EvaluateSwordfight principal {first_principal:?} for owner {entity_id:?} has no upright eye point"
                    )
                });
            range_or_los_prune = !crate::sight_obstacle::is_reachable_3d(
                self.sight_obstacles(assets),
                [self_eye.x, self_eye.y, self_eye.z],
                [principal_eye.x, principal_eye.y, principal_eye.z],
                crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
            );
        }
        if range_or_los_prune {
            self.delete_opponent(sim, assets, entity_id, first_principal);
            self.delete_opponent(sim, assets, first_principal, entity_id);
            self.evaluate_opponents(sim, assets, entity_id);
            self.evaluate_opponents(sim, assets, first_principal);
            return;
        }

        {
            let creation_order = self.world.original_creation_order(entity_id);
            if crate::combat::tiredness_debug_matches(creation_order) {
                eprintln!(
                    "RUST_TIREDNESS frame={} co={creation_order} site=weak_threshold_read \
                     tiredness={tiredness} verdict={}",
                    self.control.frame_counter,
                    if tiredness >= TIREDNESS_WEAK_THRESHOLD {
                        "tired"
                    } else {
                        "ok"
                    }
                );
            }
        }
        if tiredness >= TIREDNESS_WEAK_THRESHOLD {
            self.launch_element(crate::sequence::SequenceElement::new(
                1,
                Command::SwordstrikeTired,
                Some(entity_id),
            ));
            return;
        }

        if is_pc
            && num_opponents >= 2
            && crate::sim_rng::u32(sim, crate::sim_rng::RngSite::MeleePrincipalReshuffle, 0..3) == 0
        {
            self.choose_principal_opponent(sim, entity_id);
        }

        // ChoosePrincipalOpponent can swap the live list. Every subsequent
        // read follows the new first entry, exactly like the Original.
        let principal_id = self
            .get_entity(entity_id)
            .and_then(Entity::human_data)
            .and_then(|human| human.opponents.first().copied())
            .unwrap_or_else(|| {
                panic!("EvaluateSwordfight owner {entity_id:?} lost its principal after selection")
            });
        let principal = self.get_entity(principal_id).unwrap_or_else(|| {
            panic!(
                "EvaluateSwordfight owner {entity_id:?} selected missing principal {principal_id:?}"
            )
        });
        let principal_human = principal.human_data().unwrap_or_else(|| {
            panic!(
                "EvaluateSwordfight owner {entity_id:?} selected non-human principal {principal_id:?}"
            )
        });
        let mutual = principal_human.opponents.first().copied() == Some(entity_id);

        let mut nonmutual_gate_roll = None;
        if mutual {
            let (has_initiative, received, relative_ability) = self
                .get_entity(entity_id)
                .and_then(Entity::human_data)
                .map(|human| {
                    (
                        human.smalltalk_initiative,
                        human.received_smalltalk_initiative,
                        human.relative_fighting_ability,
                    )
                })
                .unwrap_or_else(|| {
                    panic!("EvaluateSwordfight owner {entity_id:?} lost human state")
                });
            if has_initiative {
                if received {
                    self.get_entity_mut(entity_id)
                        .and_then(Entity::human_data_mut)
                        .unwrap_or_else(|| {
                            panic!(
                                "EvaluateSwordfight owner {entity_id:?} vanished while consuming initiative"
                            )
                        })
                        .received_smalltalk_initiative = false;
                } else {
                    let loses =
                        crate::sim_rng::u32(sim, crate::sim_rng::RngSite::MeleeInitiative, 0..100)
                            <= u32::from(relative_ability);
                    if loses || self.can_he_kill_me_but_me_not(entity_id, principal_id, assets) {
                        self.get_entity_mut(entity_id)
                            .and_then(Entity::human_data_mut)
                            .unwrap_or_else(|| {
                                panic!(
                                    "EvaluateSwordfight owner {entity_id:?} vanished during initiative transfer"
                                )
                            })
                            .smalltalk_initiative = false;
                        let opponent_human = self
                            .get_entity_mut(principal_id)
                            .and_then(Entity::human_data_mut)
                            .unwrap_or_else(|| {
                                panic!(
                                    "EvaluateSwordfight owner {entity_id:?} principal {principal_id:?} vanished during initiative transfer"
                                )
                            });
                        opponent_human.smalltalk_initiative = true;
                        opponent_human.received_smalltalk_initiative = true;
                        return;
                    }
                }
            } else {
                self.update_swordfight_distance(sim, assets, entity_id);
                return;
            }
        } else {
            let roll =
                crate::sim_rng::u32(sim, crate::sim_rng::RngSite::MeleeNonMutualGate, 0..100);
            nonmutual_gate_roll = Some(roll);
            if roll >= 10 {
                let frame = self.control.frame_counter;
                if reactive_sword_debug_frame_matches(frame) {
                    let creation_order = self.world.original_creation_order(entity_id);
                    if reactive_sword_debug_creation_order_matches(creation_order) {
                        eprintln!(
                            "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=evaluate_nonmutual roll={} accepted=false is_pc={} selected_pc={}]",
                            frame,
                            creation_order,
                            entity_id.index(),
                            principal_id.index(),
                            roll,
                            is_pc,
                            is_pc && self.selected_pc_ids().contains(&entity_id),
                        );
                    }
                }
                return;
            }
        }

        let (self_pos, self_max, selected_pc) = {
            let entity = self.get_entity(entity_id).unwrap_or_else(|| {
                panic!("EvaluateSwordfight owner {entity_id:?} vanished before strike selection")
            });
            let max = required_hth_weapon_profile(
                entity,
                entity_id,
                &assets.profile_manager,
                "EvaluateSwordfight maximal range",
            )
            .distance[crate::weapons::WeaponDistance::Maximal as usize];
            (
                entity.element_data().position(),
                max,
                entity.is_pc() && self.selected_pc_ids().contains(&entity_id),
            )
        };
        let principal_pos = self
            .get_entity(principal_id)
            .unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight owner {entity_id:?} principal {principal_id:?} vanished before strike selection"
                )
            })
            .element_data()
            .position();
        let dx = principal_pos.x - self_pos.x;
        let dy = principal_pos.y - self_pos.y;
        let dz = principal_pos.z - self_pos.z;
        let near = is_within_smalltalk_strike_range(self_max, dx * dx + dy * dy + dz * dz);
        let frame = self.control.frame_counter;
        if reactive_sword_debug_frame_matches(frame) {
            let creation_order = self.world.original_creation_order(entity_id);
            if reactive_sword_debug_creation_order_matches(creation_order) {
                eprintln!(
                    "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=evaluate_gate mutual={} nonmutual_roll={:?} near={} is_pc={} selected_pc={}]",
                    frame,
                    creation_order,
                    entity_id.index(),
                    principal_id.index(),
                    mutual,
                    nonmutual_gate_roll,
                    near,
                    is_pc,
                    selected_pc,
                );
            }
        }
        if !near {
            self.update_swordfight_distance(sim, assets, entity_id);
            return;
        }

        if is_pc && !selected_pc {
            self.pc_propose_and_launch_strike(sim, assets, entity_id, principal_id);
        }

        if let Some(destination) = self.is_step_back_needed(sim, entity_id, assets) {
            let layer = self
                .get_entity(entity_id)
                .unwrap_or_else(|| {
                    panic!("EvaluateSwordfight step-back owner {entity_id:?} is missing")
                })
                .element_data()
                .layer();
            // Do not publish `last_motion_was_step_back_in_combat` merely
            // because the movement was requested. Original changes that
            // member only from Human::Execute's RHMOTION_TERMINATED arm. A
            // parry or injury can abort this Move before it reaches that arm,
            // in which case the preceding value must survive.
            self.launch_evaluated_step_back(entity_id, destination, layer);
            return;
        }

        let is_left = crate::sim_rng::bool(sim, crate::sim_rng::RngSite::SmalltalkStrikeSide);
        let command = if is_left {
            Command::SwordstrikeSmalltalkLeft
        } else {
            Command::SwordstrikeSmalltalkRight
        };
        self.register_owned_element_deferred(crate::sequence::SequenceElement::new_interaction(
            1,
            command,
            Some(entity_id),
            Some(principal_id),
        ));
        self.receive_smalltalk_hint(entity_id, principal_id, is_left);
    }

    pub(super) fn launch_evaluated_step_back(
        &mut self,
        entity_id: EntityId,
        destination: crate::coordinates::MapPoint,
        layer: u16,
    ) {
        let mut element = crate::sequence::SequenceElement::new_movement(
            1,
            Command::Move,
            Some(entity_id),
            crate::order::OrderType::WalkingUpright,
        );
        element.data = crate::sequence::SequenceElementData::Movement {
            destination,
            layer,
            sector: None,
            gate_id: None,
            line_id: None,
            element: None,
            flags: crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT,
            tolerance: 0.0,
            direction: 0,
            action: crate::order::OrderType::WalkingUpright,
            speed_factor: 1.0,
            post_seek_sequence: None,
        };
        self.launch_element(element);
    }
    /// Build the strike
    /// selection context for a non-selected PC, query
    /// `propose_good_sword_strike`, and launch the resulting strike
    /// as a sequence element interaction.  No hulk build-up, no
    /// preparation delay, no war-cry — those are soldier-only
    /// embellishments handled in `tick_enemy_sword_attacks`.
    pub(super) fn pc_propose_and_launch_strike(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        pc_id: EntityId,
        target_id: EntityId,
    ) {
        // Skip if PC already has an active strike in flight.
        let pc = self.get_entity(pc_id).unwrap_or_else(|| {
            panic!("EvaluateSwordfight strike proposal PC {pc_id:?} is missing")
        });
        let already_striking = self
            .orders
            .sequence_manager
            .current_order_for_actor(pc_id)
            .is_some_and(|(_, _, order)| sword_strike_from_animation(order.order_type).is_some());
        if already_striking {
            return;
        }

        // Read PC profile + sprite snapshots up front.
        let pc_data = match pc {
            Entity::Pc(p) => p,
            _ => panic!("EvaluateSwordfight strike proposal owner {pc_id:?} is not a PC"),
        };
        let character = assets
            .profile_manager
            .get_character(pc_data.pc.profile_index)
            .unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight strike proposal PC {pc_id:?} references missing character profile {:?}",
                    pc_data.pc.profile_index
                )
            });
        let weapon_id = character.hth_weapon_id;
        let fighting_ability = character.fighting;
        let direction = pc.element_data().direction();
        let elevation = pc.element_data().position().z;
        let attacker_pos = (
            pc.element_data().position_map().x,
            pc.element_data().position_map().y,
        );
        let mut boredom = pc_data.human.sword_strike_boredom.clone();
        let is_swordfighting = !pc_data.human.opponents.is_empty();
        let attacker_profile = assets
            .profile_manager
            .get_hth_weapon(weapon_id)
            .unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight strike proposal PC {pc_id:?} references missing hand-to-hand weapon {weapon_id}"
                )
            });

        // Sprite-derived timing data for the strike-selection context.
        let attacker_sprite_frames: Option<[i16; crate::weapons::NUM_NORMAL_SWORD_STRIKES]> = self
            .get_entity(pc_id)
            .map(|e| &e.element_data().sprite)
            .map(|sprite| {
                use crate::combat::NORMAL_STRIKES;
                let mut frames = [0i16; crate::weapons::NUM_NORMAL_SWORD_STRIKES];
                for (i, &s) in NORMAL_STRIKES.iter().enumerate() {
                    let anim = strike_to_animation(s);
                    frames[i] = sprite.frames_from_start_till_action_done(anim) as i16;
                }
                frames
            });
        let parry_startup: Option<i16> = self
            .get_entity(pc_id)
            .map(|e| &e.element_data().sprite)
            .map(|sprite| {
                sprite.frames_from_start_till_action_done(
                    crate::order::OrderType::TransitionWaitingSwordParryingSword,
                ) as i16
            });
        self.get_entity(target_id)
            .unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight strike proposal PC {pc_id:?} references missing target {target_id:?}"
                )
            })
            .actor_data()
            .unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight strike proposal PC {pc_id:?} target {target_id:?} is not an actor"
                )
            });
        let opponent_time_limit = self.opponent_sword_strike_time_limit_for_actor(pc_id, target_id);

        // Build the nearby-victim list (same shape as the soldier path).
        let inv_aspect = INVERSE_SWORDFIGHT_ASPECT_RATIO;
        let obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        let nearby: Vec<crate::combat::NearbyVictim> = self
            .world
            .entities
            .humans()
            .filter_map(|(eid, e)| {
                if eid == pc_id {
                    return None;
                }
                let elem = e.element_data();
                if !elem.active {
                    return None;
                }
                let eligible_for_regular_strikes = is_possible_sword_strike_victim(
                    &self.world.entities,
                    pc_id,
                    e,
                    eid,
                    &assets.profile_manager,
                    &self.world.fast_grid,
                    obstacles,
                );
                let vdx = elem.position_map().x - attacker_pos.0;
                let vdy = (elem.position_map().y - attacker_pos.1) * inv_aspect;
                let dist = (vdx * vdx + vdy * vdy).sqrt();
                let sector = crate::position_interface::vector_to_sector_0_to_15(vdx, vdy) as u8;
                let def_wid = get_hth_weapon_id_full(e, &assets.profile_manager);
                let def_prof = def_wid.and_then(|id| assets.profile_manager.get_hth_weapon(id));
                let lp = get_life_points(e);
                let is_walking_with_sword = e
                    .actor_data()
                    .map(|a| a.action_state == ActionState::MovingSword)
                    .unwrap_or(false);
                Some(crate::combat::NearbyVictim {
                    eligible_for_regular_strikes,
                    dx: vdx,
                    dy_stretched: vdy,
                    distance: dist,
                    direction_sector: sector,
                    camp: match e {
                        Entity::Pc(_) => crate::element::Camp::Royalists,
                        Entity::Soldier(s) => s.soldier.cached_camp,
                        Entity::Civilian(c) => c.civilian.cached_camp,
                        _ => crate::element::Camp::Error,
                    },
                    facing_direction: elem.direction(),
                    elevation: elem.position().z,
                    life_points: lp,
                    defender_profile: def_prof,
                    is_primary_target: eid == target_id,
                    is_walking_with_sword,
                })
            })
            .collect();

        let ctx = crate::combat::StrikeSelectionContext {
            attacker_profile,
            fighting_ability,
            blood_alcohol: 0, // PCs don't (currently) carry blood alcohol
            is_rank_soldier: false,
            attacker_direction: direction,
            attacker_elevation: elevation,
            attacker_camp: crate::element::Camp::Royalists,
            is_swordfighting,
            opponent_time_limit,
            strike_startup_frames: attacker_sprite_frames,
            parry_startup_frames: parry_startup,
            is_npc: false,
        };

        let frame = self.control.frame_counter;
        let debug = reactive_sword_debug_frame_matches(frame)
            .then(|| {
                let creation_order = self.world.original_creation_order(pc_id);
                reactive_sword_debug_creation_order_matches(creation_order).then_some(
                    crate::combat::SwordStrikeProposalDebug {
                        frame,
                        victim: pc_id.index(),
                        victim_creation_order: creation_order,
                        attacker: target_id.index(),
                    },
                )
            })
            .flatten();
        let rng_before = debug.and_then(|_| self.control.rng.original_replay_cursor());
        let mut sweep_rebase = None;
        let proposed = crate::combat::propose_good_sword_strike_with_debug(
            sim,
            &ctx,
            &nearby,
            &mut boredom,
            false,
            debug,
            &mut sweep_rebase,
        );
        self.apply_strike_selection_sweep_rebase(assets, pc_id, sweep_rebase);
        if let Some(debug) = debug {
            eprintln!(
                "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=proposal_boundary caller=pc_evaluate rng_before={:?} rng_after={:?} result={:?}]",
                debug.frame,
                debug.victim_creation_order,
                debug.victim,
                debug.attacker,
                rng_before,
                self.control.rng.original_replay_cursor(),
                proposed,
            );
        }

        // ProposeGoodSwordStrike mutates the live boredom counters while
        // evaluating candidates, even when no strike is ultimately viable.
        // Persist those decrements before the no-proposal return.
        if let Some(entity) = self.world.entities.get_mut(pc_id)
            && let Some(human) = entity.human_data_mut()
        {
            human.sword_strike_boredom = boredom;
        }

        let strike = match proposed {
            Some(crate::combat::ProposedCombatAction::Strike(s)) => s,
            _ => return,
        };

        // Launch the strike as a per-target interaction.
        let cmd = strike.to_command();
        let elem =
            crate::sequence::SequenceElement::new_interaction(1, cmd, Some(pc_id), Some(target_id));
        self.launch_element(elem);
    }

    /// Decides whether the actor should take a one-step backward
    /// walk during smalltalk swordfight to break an encirclement
    /// when the forward-facing opponents outweigh our "friends"
    /// ability.  Returns `Some(destination)` when the step-back
    /// should fire, or `None` otherwise.
    ///
    /// Guards:
    /// - Selected PC — the player drives its own movement.
    /// - Soldier (any rank) — only non-soldier humans step back; the
    ///   soldier AI owns its own retreat.
    /// - NPC in line formation (soldier with a combat neighbour) —
    ///   only reachable in theory via non-soldier branches, kept for
    ///   parity.
    ///
    /// Scoring: sum the fighting ability of every opponent in front
    /// of us (positive dot product with our facing vector).
    /// Opponents inside `max(my_max_range, their_max_range)`
    /// contribute their full ability; the rest contribute 1.
    /// Compare against the principal opponent's own opponents
    /// (i.e. our friends) using the roll
    /// `(rand() % 100) * opponents_ability <= 100 * friends_ability`.
    ///
    /// Reachability: the step-back destination is
    /// `position_map + dir * -abs(backward_walk_distance)` and must
    /// satisfy `is_straight_movement_authorized`.
    pub(super) fn is_step_back_needed(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        entity_id: impl Into<EntityId>,
        assets: &LevelAssets,
    ) -> Option<crate::coordinates::MapPoint> {
        let entity_id = entity_id.into();
        let entity = self.get_entity(entity_id).unwrap_or_else(|| {
            panic!("EvaluateSwordfight step-back owner {entity_id:?} is missing")
        });

        if entity.is_pc() && self.selected_pc_ids().contains(&entity_id) {
            return None;
        }
        if entity.is_soldier() {
            return None;
        }
        if let Some(ai) = entity.enemy_ai()
            && (ai.left_combat_neighbour != 0 || ai.right_combat_neighbour != 0)
        {
            return None;
        }

        let opponents: Vec<EntityId> = entity
            .human_data()
            .unwrap_or_else(|| {
                panic!("EvaluateSwordfight step-back owner {entity_id:?} is not human")
            })
            .opponents
            .ids();
        if opponents.is_empty() {
            return None;
        }
        let principal_id = *opponents
            .first()
            .expect("nonempty opponent list checked above");

        // `uwFriendsAbility = GetPrincipalOpponent()->GetOpponentsFightingAbility()`
        let friends = self
            .get_entity(principal_id)
            .unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight step-back owner {entity_id:?} references missing principal {principal_id:?}"
                )
            })
            .human_data()
            .unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight step-back principal {principal_id:?} for {entity_id:?} is not human"
                )
            })
            .opponents
            .ids();
        let friends_ability: u16 = friends
            .iter()
            .map(|id| {
                self.get_entity(*id).unwrap_or_else(|| {
                    panic!(
                        "EvaluateSwordfight step-back principal {principal_id:?} references missing friend {id:?}"
                    )
                })
            })
            .map(|e| {
                fighting_ability_from_profile(
                    e,
                    &assets.profile_manager,
                    self.control.sim_config.difficulty,
                )
            })
            .fold(0u16, |acc, fa| acc.saturating_add(fa));

        // RHElement::GetDirectionVector calls SetSector0to15 with
        // ASPECT_RATIO. Keep this in projected map space for both the
        // forward-facing test and the eventual step-back destination.
        let [dx_dir, dy_dir] =
            crate::position_interface::sector_to_vector_iso(entity.element_data().direction());
        let my_pos = entity.element_data().position();

        let my_max_range = required_hth_weapon_profile(
            entity,
            entity_id,
            &assets.profile_manager,
            "EvaluateSwordfight step-back range",
        )
        .distance[crate::weapons::WeaponDistance::Maximal as usize]
            as f32;

        let mut opponents_ability: u16 = 0;
        for opp_id in opponents.iter().copied() {
            let opp = self.get_entity(opp_id).unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight step-back owner {entity_id:?} references missing opponent {opp_id:?}"
                )
            });
            let opp_pos = opp.element_data().position();
            let rel_x = opp_pos.x - my_pos.x;
            let rel_y = (opp_pos.y - my_pos.y) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
            // Forward filter: dot(dir, rel) >= 0
            if dx_dir * rel_x + dy_dir * rel_y < 0.0 {
                continue;
            }
            let opp_max_range = required_hth_weapon_profile(
                opp,
                opp_id,
                &assets.profile_manager,
                "EvaluateSwordfight opponent step-back range",
            )
            .distance[crate::weapons::WeaponDistance::Maximal as usize]
                as f32;
            let max_range = my_max_range.max(opp_max_range);
            let sq_range = max_range * max_range;
            let sq_dist = rel_x * rel_x + rel_y * rel_y;
            if sq_dist <= sq_range {
                let fa = fighting_ability_from_profile(
                    opp,
                    &assets.profile_manager,
                    sim.config().difficulty,
                );
                opponents_ability = opponents_ability.saturating_add(fa);
            } else {
                opponents_ability = opponents_ability.saturating_add(1);
            }
        }

        // `(rand() % 100) * uwOpponentsAbility <= 100 * uwFriendsAbility`
        let roll = crate::sim_rng::u32(sim, crate::sim_rng::RngSite::MeleeStepBack, 0..100) as u64;
        if roll * opponents_ability as u64 <= 100u64 * friends_ability as u64 {
            return None;
        }

        // Reachability check.  Walk
        // `-abs(distance_for_animation(WalkingBackwardsSword))`
        // along the facing direction so the destination is behind us.
        let backward_dist = (entity
            .element_data()
            .sprite
            .distance_for_animation(crate::order::OrderType::WalkingBackwardsSword)
            as f32)
            .abs();
        let my_map = entity.element_data().position_map();
        let dest = crate::coordinates::MapPoint {
            x: my_map.x - backward_dist * dx_dir,
            y: my_map.y - backward_dist * dy_dir,
        };
        let layer = entity.element_data().layer();
        let move_box = *entity.position_iface().get_move_box();
        if !self
            .world
            .fast_grid
            .is_straight_movement_authorized(my_map, dest, layer, &move_box)
        {
            return None;
        }

        Some(dest)
    }

    /// Returns `true` when the opponent is in striking range of us but we
    /// are *not* in striking range of them — i.e. we'd get hit before our
    /// strike lands, so we should defer the smalltalk strike and let them
    /// take initiative.  Uses Y-stretched distance (isometric-aware) and
    /// each combatant's MAXIMAL sword range.
    pub(super) fn can_he_kill_me_but_me_not(
        &self,
        me_id: impl Into<EntityId>,
        opponent_id: impl Into<EntityId>,
        assets: &LevelAssets,
    ) -> bool {
        let me_id = me_id.into();
        let opponent_id = opponent_id.into();
        let me = self.get_entity(me_id).unwrap_or_else(|| {
            panic!("EvaluateSwordfight range comparison owner {me_id:?} is missing")
        });
        let opponent = self.get_entity(opponent_id).unwrap_or_else(|| {
            panic!(
                "EvaluateSwordfight range comparison opponent {opponent_id:?} for {me_id:?} is missing"
            )
        });
        let me_pos = me.element_data().position();
        let opp_pos = opponent.element_data().position();

        let dx = opp_pos.x - me_pos.x;
        let dy = (opp_pos.y - me_pos.y) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
        let dz = opp_pos.z - me_pos.z;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();

        let my_max_range = required_hth_weapon_profile(
            me,
            me_id,
            &assets.profile_manager,
            "EvaluateSwordfight initiative range",
        )
        .distance[crate::weapons::WeaponDistance::Maximal as usize]
            as f32;
        let opp_max_range = required_hth_weapon_profile(
            opponent,
            opponent_id,
            &assets.profile_manager,
            "EvaluateSwordfight opponent initiative range",
        )
        .distance[crate::weapons::WeaponDistance::Maximal as usize]
            as f32;

        dist > my_max_range && dist < opp_max_range
    }

    /// Warn potential victims of an incoming strike so they can auto-parry
    /// or (for NPC soldiers) consider a reactive parry/counter-strike.
    ///
    /// - **PCs**: simple auto-parry via the worth-parry skill check.
    /// - **NPC soldiers**: dispatches EventSwordstrike →
    ///   `consider_to_begin_parade`.
    ///
    /// Selected PCs never auto-parry (the player controls their parry).
    pub(super) fn warn_for_strike(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        attacker_id: EntityId,
        victims: &[EntityId],
        strike: SwordStrike,
    ) {
        for &victim_id in victims {
            #[cfg(test)]
            record_strike_warning(attacker_id, victim_id);
            // Check what kind of victim this is and their state
            let victim_info = {
                let victim = match self.get_entity(victim_id) {
                    Some(e) => e,
                    None => continue,
                };
                let is_selected_pc =
                    victim.kind().is_pc() && self.selected_pc_ids().contains(&victim_id);
                let is_npc_soldier = matches!(victim, Entity::Soldier(_));
                let npc_substate = if let Entity::Soldier(s) = victim {
                    Some(s.npc.ai_substate())
                } else {
                    None
                };
                let ability = fighting_ability_from_profile(
                    victim,
                    &assets.profile_manager,
                    sim.config().difficulty,
                );
                let is_swordfighting = victim
                    .human_data()
                    .is_some_and(|human| !human.opponents.is_empty());
                (
                    is_selected_pc,
                    is_npc_soldier,
                    npc_substate,
                    ability,
                    is_swordfighting,
                )
            };
            let (is_selected_pc, is_npc_soldier, npc_substate, ability, is_swordfighting) =
                victim_info;

            if is_selected_pc {
                // Player controls parry for selected PCs
                continue;
            }

            // NPC soldiers use the consider-to-begin-parade AI path
            // if in a swordfight substate.  WarnForStrike on soldiers
            // ONLY dispatches EventSwordstrike — no auto-parry
            // fallback — so soldiers not in these substates simply
            // get nothing.
            if is_npc_soldier {
                let in_swordfight_substate = matches!(
                    npc_substate,
                    Some(crate::ai::Substate::AttackingSwordfight)
                        | Some(crate::ai::Substate::AttackingSwordfightSpecialStrike)
                        | Some(crate::ai::Substate::AttackingMovingAroundOldEnemy)
                        | Some(crate::ai::Substate::AttackingApproachingNewEnemy)
                );
                let frame = self.control.frame_counter;
                let debug = reactive_sword_debug_frame_matches(frame)
                    .then(|| {
                        let creation_order = self.world.original_creation_order(victim_id);
                        reactive_sword_debug_creation_order_matches(creation_order)
                            .then_some(creation_order)
                    })
                    .flatten();
                if reactive_sword_debug_frame_matches(frame) {
                    let creation_order = self.world.original_creation_order(victim_id);
                    if reactive_sword_debug_creation_order_matches(creation_order) {
                        let known =
                            self.get_entity(victim_id)
                                .and_then(Entity::enemy_ai)
                                .map(|ai| {
                                    [
                                        ai.known_enemy_strike_1,
                                        ai.known_enemy_strike_2,
                                        ai.known_enemy_strike_3,
                                    ]
                                });
                        let attacker_command_strike = self
                            .orders
                            .sequence_manager
                            .current_element_for_actor(attacker_id)
                            .and_then(|(seq_id, elem_idx)| {
                                self.orders.sequence_manager.get_element(seq_id, elem_idx)
                            })
                            .and_then(|element| SwordStrike::from_command(element.command));
                        eprintln!(
                            "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=warning strike={:?} attacker_command_strike={:?} substate={:?} swordfighting={} accepted_substate={} known={:?}]",
                            frame,
                            creation_order,
                            victim_id.index(),
                            attacker_id.index(),
                            strike,
                            attacker_command_strike,
                            npc_substate,
                            is_swordfighting,
                            in_swordfight_substate,
                            known,
                        );
                    }
                }
                let scratch = self.build_owner_context_scratch_without_forecast(assets);
                let victim = self
                    .world
                    .entities
                    .get(victim_id)
                    .expect("sword-strike warning victim disappeared");
                let mut ctx = crate::engine::ai::build_ai_context_from_entity(
                    victim,
                    frame,
                    self.entity_building_sector(victim.element_data().sector()),
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                );
                self.refresh_selected_default_wait_identity(victim_id, &mut ctx);
                let tick =
                    self.build_npc_tick_data_without_forecasts(sim, victim_id, &scratch, assets);
                let stimulus = crate::ai::Stimulus::with_human(
                    crate::ai::StimulusType::EventSwordStrike,
                    attacker_id.index(),
                );
                let rng_before = debug.and_then(|_| self.control.rng.original_replay_cursor());
                self.dispatch_filtered_stimulus_without_forecast(
                    sim, assets, victim_id, &stimulus, &ctx, &tick,
                );
                if let Some(creation_order) = debug {
                    eprintln!(
                        "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=warning_return rng_before={:?} rng_after={:?}]",
                        frame,
                        creation_order,
                        victim_id.index(),
                        attacker_id.index(),
                        rng_before,
                        self.control.rng.original_replay_cursor(),
                    );
                }
                continue;
            }

            // PC parade/counter-strike via
            // `propose_good_sword_strike(sim, also_parade=true)`, which
            // may produce a counter-strike or a parry fallback.
            //
            // Original `IsSwordfighting()` is relationship-based:
            // `mlistOpponents.Size() != 0`. Merely retaining a sword action
            // after the duel link was cleared does not permit an automatic
            // parry/counter-strike.
            if !is_swordfighting {
                continue;
            }

            // "Already striking" short-circuit: read the actor's
            // current sequence command and skip if it's any
            // sword-strike command.
            let already_striking = self
                .orders
                .sequence_manager
                .current_element_for_actor(victim_id)
                .and_then(|(seq, idx)| self.orders.sequence_manager.get_element(seq, idx))
                .map(|e| e.command.is_swordstrike())
                .unwrap_or(false);
            if already_striking {
                continue;
            }

            // Get PC weapon profile, boredom, and principal
            // opponent.  The counter-strike target is the PC's
            // `melee_target` (its current duel partner) — fall back
            // to the incoming `attacker_id` when no duel partner is
            // set.
            let (
                pc_weapon_id,
                pc_camp,
                pc_direction,
                pc_elevation,
                mut pc_boredom,
                pc_pos,
                principal_opponent,
            ) = {
                let Some(entity) = self.world.entities.get(victim_id) else {
                    continue;
                };
                let wid = get_hth_weapon_id_full(entity, &assets.profile_manager);
                let camp = match entity {
                    Entity::Pc(_) => crate::element::Camp::Royalists,
                    _ => crate::element::Camp::Error,
                };
                let dir = entity.element_data().direction();
                let elev = entity.element_data().position().z;
                let boredom = entity
                    .human_data()
                    .map(|h| h.sword_strike_boredom.clone())
                    .unwrap_or_default();
                let pos = entity.element_data().position_map();
                // RHElementActorPC::WarnForStrike launches a counter-strike
                // against GetPrincipalOpponent(), i.e. the first entry in
                // the live ordered opponent list.  The UI-facing PC
                // `melee_target` cache can lag behind a same-frame principal
                // reshuffle and must not drive combat selection.
                let principal = entity
                    .human_data()
                    .and_then(|human| human.opponents.first().copied());
                (wid, camp, dir, elev, boredom, (pos.x, pos.y), principal)
            };

            let pc_profile =
                match pc_weapon_id.and_then(|id| assets.profile_manager.get_hth_weapon(id)) {
                    Some(p) => p,
                    None => continue,
                };

            // Read strike startup frames from the PC's sprite data.
            let pc_sprite_frames: Option<[i16; crate::weapons::NUM_NORMAL_SWORD_STRIKES]> = self
                .get_entity(victim_id)
                .map(|e| &e.element_data().sprite)
                .map(|sprite| {
                    use crate::combat::NORMAL_STRIKES;
                    let mut frames = [0i16; crate::weapons::NUM_NORMAL_SWORD_STRIKES];
                    for (i, &s) in NORMAL_STRIKES.iter().enumerate() {
                        let anim = strike_to_animation(s);
                        frames[i] = sprite.frames_from_start_till_action_done(anim) as i16;
                    }
                    frames
                });
            let pc_parry_startup: Option<i16> = self
                .get_entity(victim_id)
                .map(|e| &e.element_data().sprite)
                .map(|sprite| {
                    sprite.frames_from_start_till_action_done(
                        crate::order::OrderType::TransitionWaitingSwordParryingSword,
                    ) as i16
                });

            let frame = self.control.frame_counter;
            let debug = reactive_sword_debug_frame_matches(frame)
                .then(|| {
                    let victim_creation_order = self.world.original_creation_order(victim_id);
                    reactive_sword_debug_creation_order_matches(victim_creation_order).then_some(
                        crate::combat::SwordStrikeProposalDebug {
                            frame,
                            victim: victim_id.index(),
                            victim_creation_order,
                            attacker: attacker_id.index(),
                        },
                    )
                })
                .flatten();
            if let Some(debug) = debug {
                let sprite = &self
                    .get_entity(victim_id)
                    .expect("reactive sword timing debug victim disappeared")
                    .element_data()
                    .sprite;
                let action = crate::order::OrderType::TransitionWaitingSwordParryingSword;
                let timing =
                    |conversion: &[u16], scripts: &[crate::sprite_script::SpriteScript]| {
                        let row = conversion.get(action as usize).copied()?;
                        if row == crate::sprite_script::UNMAPPED {
                            return None;
                        }
                        let script = scripts.get(row as usize)?;
                        let frame_count = script.frame_ids.len();
                        let waits = script
                            .delays
                            .iter()
                            .copied()
                            .take((script.action_done as usize + 1).min(frame_count))
                            .collect::<Vec<_>>();
                        let startup = waits.iter().copied().fold(0u16, u16::saturating_add);
                        Some((row, script.action_done, frame_count, waits, startup))
                    };
                let primary = timing(&sprite.conversion, &sprite.scripts);
                let alternate = sprite
                    .alternate_conversion
                    .as_deref()
                    .zip(sprite.alternate_scripts.as_deref())
                    .and_then(|(conversion, scripts)| timing(conversion, scripts));
                eprintln!(
                    "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=parry_timing active_alternate={} primary_key={:?} alternate_key={:?} current={:?} primary={:?} alternate={:?}]",
                    debug.frame,
                    debug.victim_creation_order,
                    debug.victim,
                    debug.attacker,
                    sprite.use_alternate_profile,
                    sprite.profile_cache_key,
                    sprite.alternate_profile_cache_key,
                    if sprite.use_alternate_profile {
                        alternate.as_ref()
                    } else {
                        primary.as_ref()
                    },
                    primary,
                    alternate,
                );
            }

            // RHElementActorHuman::ProposeGoodSwordStrike always limits a
            // reactive counter-strike by the principal opponent's remaining
            // strike time. Without this deadline a warned PC can choose a
            // counter-strike that cannot land before the incoming blow, where
            // Original rejects every strike and falls back to ParrySword.
            let target_id_for_nearby = principal_opponent.unwrap_or(attacker_id);
            let opponent_time_limit = {
                let opponent = self.get_entity(target_id_for_nearby).unwrap_or_else(|| {
                    panic!(
                        "WarnForStrike PC {victim_id:?} references missing principal opponent {target_id_for_nearby:?}"
                    )
                });
                opponent.actor_data().unwrap_or_else(|| {
                    panic!(
                        "WarnForStrike PC {victim_id:?} principal opponent {target_id_for_nearby:?} is not an actor"
                    )
                });
                let sprite = &opponent.element_data().sprite;
                let principal_animation = self.live_actor_animation(target_id_for_nearby);
                let raw_frames = debug.map(|_| sprite.frames_from_now_till_action_done());
                let time_limit = self
                    .opponent_sword_strike_time_limit_for_actor(victim_id, target_id_for_nearby)
                    .unwrap_or(1000);
                if let (Some(debug), Some(raw_frames)) = (debug, raw_frames) {
                    eprintln!(
                        "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=pc_principal principal={} animation={:?} raw_frames_from_now={} time_limit={}]",
                        debug.frame,
                        debug.victim_creation_order,
                        debug.victim,
                        debug.attacker,
                        target_id_for_nearby.index(),
                        principal_animation,
                        raw_frames,
                        time_limit,
                    );
                }
                Some(time_limit)
            };

            // Build nearby victims so circle/push/round strike scoring can
            // see adjacent enemies — same shape as the strike-launcher and
            // PC strike-propose paths.
            let inv_aspect = INVERSE_SWORDFIGHT_ASPECT_RATIO;
            let obstacles = crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.static_sight_obstacles.as_slice(),
                dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                static_active: &self.world.static_sight_obstacle_active,
            };
            let nearby: Vec<crate::combat::NearbyVictim> = self
                .world
                .entities
                .humans()
                .filter_map(|(eid, e)| {
                    if eid == victim_id {
                        return None;
                    }
                    let elem = e.element_data();
                    if !elem.active {
                        return None;
                    }
                    let eligible_for_regular_strikes = is_possible_sword_strike_victim(
                        &self.world.entities,
                        victim_id,
                        e,
                        eid,
                        &assets.profile_manager,
                        &self.world.fast_grid,
                        obstacles,
                    );
                    let vdx = elem.position_map().x - pc_pos.0;
                    let vdy = (elem.position_map().y - pc_pos.1) * inv_aspect;
                    let dist = (vdx * vdx + vdy * vdy).sqrt();
                    let sector =
                        crate::position_interface::vector_to_sector_0_to_15(vdx, vdy) as u8;
                    let def_wid = get_hth_weapon_id_full(e, &assets.profile_manager);
                    let def_prof = def_wid.and_then(|id| assets.profile_manager.get_hth_weapon(id));
                    let lp = get_life_points(e);
                    let is_walking_with_sword = e
                        .actor_data()
                        .map(|a| a.action_state == ActionState::MovingSword)
                        .unwrap_or(false);
                    Some(crate::combat::NearbyVictim {
                        eligible_for_regular_strikes,
                        dx: vdx,
                        dy_stretched: vdy,
                        distance: dist,
                        direction_sector: sector,
                        camp: match e {
                            Entity::Pc(_) => crate::element::Camp::Royalists,
                            Entity::Soldier(s) => s.soldier.cached_camp,
                            Entity::Civilian(c) => c.civilian.cached_camp,
                            _ => crate::element::Camp::Error,
                        },
                        facing_direction: elem.direction(),
                        elevation: elem.position().z,
                        life_points: lp,
                        defender_profile: def_prof,
                        is_primary_target: eid == target_id_for_nearby,
                        is_walking_with_sword,
                    })
                })
                .collect();

            let strike_ctx = crate::combat::StrikeSelectionContext {
                attacker_profile: pc_profile,
                fighting_ability: ability,
                blood_alcohol: 0,
                is_rank_soldier: false,
                attacker_direction: pc_direction,
                attacker_elevation: pc_elevation,
                attacker_camp: pc_camp,
                is_swordfighting,
                opponent_time_limit,
                strike_startup_frames: pc_sprite_frames,
                parry_startup_frames: pc_parry_startup,
                is_npc: false, // PC path — different skill gate
            };

            let rng_before = debug.and_then(|_| self.control.rng.original_replay_cursor());
            let mut sweep_rebase = None;
            let proposed = crate::combat::propose_good_sword_strike_with_debug(
                sim,
                &strike_ctx,
                &nearby,
                &mut pc_boredom,
                true, // also_parade
                debug,
                &mut sweep_rebase,
            );
            self.apply_strike_selection_sweep_rebase(assets, victim_id, sweep_rebase);
            if let Some(debug) = debug {
                eprintln!(
                    "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=proposal_boundary caller=pc_reactive_warning rng_before={:?} rng_after={:?} result={:?}]",
                    debug.frame,
                    debug.victim_creation_order,
                    debug.victim,
                    debug.attacker,
                    rng_before,
                    self.control.rng.original_replay_cursor(),
                    proposed,
                );
            }

            // Write back boredom
            if let Some(entity) = self.world.entities.get_mut(victim_id)
                && let Some(human) = entity.human_data_mut()
            {
                human.sword_strike_boredom = pc_boredom;
            }

            match proposed {
                Some(crate::combat::ProposedCombatAction::Parry) => {
                    // Original ConsiderSwordAttack runs from the current
                    // WaitingSword Execute callback. LaunchSequenceElement
                    // only registers ParrySword here; its Instruct/priority
                    // arbitration happens in SequenceManager::Hourglass after
                    // every actor slot. In particular, an already-selected
                    // smalltalk strike still enters Motion START (and may
                    // choose a remark) before ParrySword replaces it.
                    let parry_elem = crate::sequence::SequenceElement::new(
                        1,
                        crate::element::Command::ParrySword,
                        Some(victim_id),
                    );
                    self.trace_reactive_sword_topology(
                        "before_parry_registration",
                        victim_id,
                        None,
                    );
                    let parry_sequence = self.register_owned_element_deferred(parry_elem);
                    self.trace_reactive_sword_topology(
                        "after_parry_registration",
                        victim_id,
                        Some((parry_sequence, 0)),
                    );
                }
                Some(crate::combat::ProposedCombatAction::Strike(counter_strike)) => {
                    // PC counter-strike: launch the strike sequence
                    // against the PC's duel partner — fall back to
                    // the incoming attacker when there's no current
                    // opponent.
                    let counter_cmd = counter_strike.to_command();
                    let target = target_id_for_nearby;
                    let mut seq = crate::sequence::Sequence::new();
                    let strike_elem = crate::sequence::SequenceElement::new_interaction(
                        1,
                        counter_cmd,
                        Some(victim_id),
                        Some(target),
                    );
                    seq.append_element(strike_elem);
                    self.launch_sequence(seq);
                    tracing::debug!(
                        ?victim_id,
                        ?attacker_id,
                        ?target,
                        ?counter_strike,
                        "PC auto-parade: counter-strike"
                    );
                }
                None => {
                    // No viable action
                }
            }
        }
    }

    /// Reactive parry/counter-strike system for NPC soldiers.
    ///
    /// When a soldier receives an incoming sword strike, they consider
    /// whether to parry, dodge backward, or launch a counter-strike.
    ///
    /// Key elements:
    /// - **Known-strike memory**: soldiers remember recent strikes; if the
    ///   incoming strike isn't recognized, they do nothing.
    /// - **Step-back dodge**: skilled soldiers may dodge backward instead of
    ///   parrying push/circle strikes.
    /// - **Counter-strike**: `propose_good_sword_strike(sim, also_parade=true)`
    ///   may pick an offensive counter-attack.
    /// - **Parade fallback**: if no good counter-strike, fall into parry stance.
    pub(crate) fn consider_to_begin_parade(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        attacker_id: EntityId,
        attacker_command_strike: Option<SwordStrike>,
        animation_strike: SwordStrike,
    ) {
        // Keep the disabled diagnostic path ahead of every new simulation
        // read. Both filters are required and parsed eagerly when enabled.
        let step_back_debug = reactive_step_back_debug_config();
        // ── 1. Check if the victim recognizes this strike ────────────
        // Original compares pHitter->GetCommand() with the three command-valued
        // memory slots. It does not use the animation-derived strike here.
        let Some(command_strike) = attacker_command_strike else {
            let frame = self.control.frame_counter;
            if reactive_sword_debug_frame_matches(frame) {
                let creation_order = self.world.original_creation_order(victim_id);
                if reactive_sword_debug_creation_order_matches(creation_order) {
                    eprintln!(
                        "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=recognition accepted=false reason=no_selected_strike animation_strike={:?}]",
                        frame,
                        creation_order,
                        victim_id.index(),
                        attacker_id.index(),
                        animation_strike,
                    );
                }
            }
            return;
        };
        let is_known = {
            let Some(Entity::Soldier(s)) = self.world.entities.get(victim_id) else {
                return;
            };
            let Some(ai) = s.npc.ai_brain.enemy() else {
                return;
            };
            Some(command_strike) == ai.known_enemy_strike_1
                || Some(command_strike) == ai.known_enemy_strike_2
                || Some(command_strike) == ai.known_enemy_strike_3
        };
        if !is_known {
            let frame = self.control.frame_counter;
            if reactive_sword_debug_frame_matches(frame) {
                let creation_order = self.world.original_creation_order(victim_id);
                if reactive_sword_debug_creation_order_matches(creation_order) {
                    eprintln!(
                        "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=recognition accepted=false reason=unknown command_strike={:?} animation_strike={:?}]",
                        frame,
                        creation_order,
                        victim_id.index(),
                        attacker_id.index(),
                        command_strike,
                        animation_strike,
                    );
                }
            }
            return;
        }
        let frame = self.control.frame_counter;
        if reactive_sword_debug_frame_matches(frame) {
            let creation_order = self.world.original_creation_order(victim_id);
            if reactive_sword_debug_creation_order_matches(creation_order) {
                eprintln!(
                    "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=recognition accepted=true command_strike={:?} animation_strike={:?}]",
                    frame,
                    creation_order,
                    victim_id.index(),
                    attacker_id.index(),
                    command_strike,
                    animation_strike,
                );
            }
        }

        // ── 2. Record this strike experience (promote to head of list).
        self.make_bad_sword_strike_experience(assets, victim_id, command_strike, true);

        // ── 3. Determine push-back distance from attacker's weapon ──
        // PushAside, FalseCircle, TrueCircle → strike's maximal
        // distance; others → 0.
        let push_back_distance: u16 = {
            let attacker_weapon_id = self
                .get_entity(attacker_id)
                .and_then(|e| get_hth_weapon_id_full(e, &assets.profile_manager));
            attacker_weapon_id
                .and_then(|wid| {
                    let profile = assets.profile_manager.get_hth_weapon(wid)?;
                    let kind = profile.thrusts.get(animation_strike as usize)?.kind;
                    match kind {
                        WeaponThrustKind::PushAside
                        | WeaponThrustKind::FalseCircle
                        | WeaponThrustKind::TrueCircle => {
                            Some(profile.thrusts[animation_strike as usize].maximal_distance)
                        }
                        _ => Some(0),
                    }
                })
                .unwrap_or(0)
        };

        // ── 4. Build context and call
        //   `propose_good_sword_strike(sim, also_parade=true)`. ──
        // Collect victim state for strike selection
        let (
            victim_weapon_id,
            victim_fighting_ability,
            victim_blood_alcohol,
            victim_is_rank_soldier,
            victim_direction,
            victim_elevation,
            victim_camp,
            victim_pos,
            victim_layer,
            mut victim_boredom,
            principal_opponent,
        ) = {
            let Some(victim_entity @ Entity::Soldier(s)) = self.world.entities.get(victim_id)
            else {
                return;
            };
            let ai = match &s.npc.ai_brain {
                crate::element::AiBrain::Enemy(ai) => ai,
                _ => return,
            };
            let spi = s.soldier.soldier_profile_index;
            let sp = assets.profile_manager.get_soldier(spi).unwrap_or_else(|| {
                panic!(
                    "ConsiderToBeginParade victim {victim_id:?} references missing soldier profile {spi}"
                )
            });
            // ProposeGoodSwordStrike calls the virtual GetFightingAbility.
            // RHElementActorSoldier overrides it to apply the active
            // difficulty modifier for Lacklandists, including this reactive
            // counter-strike path.
            let fa = fighting_ability_from_profile(
                victim_entity,
                &assets.profile_manager,
                sim.config().difficulty,
            );
            let is_rank = sp.rank == crate::profiles::ProfileRank::Soldier;
            let ba = ai.base.blood_alcohol;
            let camp = s.soldier.cached_camp;
            let pos = s.element.position_map();
            let elev = s.element.position().z;
            let layer = s.element.layer();
            let dir = s.element.direction();
            let boredom = s.human.sword_strike_boredom.clone();
            // The strike proposal times and aims the victim's *principal
            // opponent* — the head of its own opponent list — not the
            // AI-picked primary target. The two differ routinely: the primary
            // target survives the end of a duel and can name a soldier this
            // victim is not fighting, while the opponent list is the live
            // duel. Straight strikes reach nobody else, so aiming the wrong
            // one silently zeroes out every straight-strike counter.
            let principal = s.human.opponents.first().copied();
            (
                ai.hth_weapon_id,
                fa,
                ba,
                is_rank,
                dir,
                elev,
                camp,
                pos,
                layer,
                boredom,
                principal,
            )
        };

        let victim_profile = match assets.profile_manager.get_hth_weapon(victim_weapon_id) {
            Some(p) => p,
            None => return,
        };

        // Collect nearby entities for strike damage estimation
        // (victim's perspective).  Y is stretched by
        // `INVERSE_SWORDFIGHT_ASPECT_RATIO` (= 1.0 in the shipping
        // game).
        let inv_aspect = INVERSE_SWORDFIGHT_ASPECT_RATIO;
        let obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        let frame = self.control.frame_counter;
        let debug = reactive_sword_debug_frame_matches(frame)
            .then(|| {
                let victim_creation_order = self.world.original_creation_order(victim_id);
                reactive_sword_debug_creation_order_matches(victim_creation_order).then_some(
                    crate::combat::SwordStrikeProposalDebug {
                        frame,
                        victim: victim_id.index(),
                        victim_creation_order,
                        attacker: attacker_id.index(),
                    },
                )
            })
            .flatten();
        let mut nearby_debug_index = debug.map(|_| 0_usize);
        let nearby: Vec<crate::combat::NearbyVictim> = self
            .world
            .entities
            .humans()
            .filter_map(|(eid, e)| {
                if eid == victim_id || !e.is_active() {
                    return None;
                }
                let elem = e.element_data();
                let eligible_for_regular_strikes = is_possible_sword_strike_victim(
                    &self.world.entities,
                    victim_id,
                    e,
                    eid,
                    &assets.profile_manager,
                    &self.world.fast_grid,
                    obstacles,
                );
                let vdx = elem.position_map().x - victim_pos.x;
                let vdy = (elem.position_map().y - victim_pos.y) * inv_aspect;
                let dist = (vdx * vdx + vdy * vdy).sqrt();
                let sector = crate::position_interface::vector_to_sector_0_to_15(vdx, vdy) as u8;
                let def_wid = match e {
                    Entity::Pc(pc) => assets
                        .profile_manager
                        .get_character(pc.pc.profile_index)
                        .map(|p| p.hth_weapon_id),
                    Entity::Soldier(s) => assets
                        .profile_manager
                        .get_soldier(s.soldier.soldier_profile_index)
                        .map(|p| p.hth_weapon_id),
                    _ => None,
                };
                let def_prof = def_wid.and_then(|id| assets.profile_manager.get_hth_weapon(id));
                let lp = get_life_points(e);
                let is_walking_with_sword = e
                    .actor_data()
                    .map(|a| a.action_state == ActionState::MovingSword)
                    .unwrap_or(false);
                let nearby = crate::combat::NearbyVictim {
                        eligible_for_regular_strikes,
                        dx: vdx,
                        dy_stretched: vdy,
                        distance: dist,
                        direction_sector: sector,
                        camp: match e {
                            Entity::Pc(_) => crate::element::Camp::Royalists,
                            Entity::Soldier(s) => s.soldier.cached_camp,
                            Entity::Civilian(c) => c.civilian.cached_camp,
                            _ => crate::element::Camp::Error,
                        },
                        facing_direction: elem.direction(),
                        elevation: elem.position().z,
                        life_points: lp,
                        defender_profile: def_prof,
                        is_primary_target: principal_opponent.is_some_and(|p| eid == p),
                    is_walking_with_sword,
                };
                if let (Some(debug), Some(index)) = (debug, nearby_debug_index.as_mut()) {
                    eprintln!(
                        "[REACTIVE_SWORD frame={} co={} victim={} phase=nearby_owner index={} target={}]",
                        debug.frame,
                        debug.victim_creation_order,
                        debug.victim,
                        *index,
                        eid.index(),
                    );
                    *index += 1;
                }
                Some(nearby)
            })
            .collect();

        // ProposeGoodSwordStrike always times the defender's principal
        // opponent, which need not be the hitter that triggered this
        // reactive warning.
        let opponent_time_limit: Option<i16> = principal_opponent.and_then(|opponent| {
            self.opponent_sword_strike_time_limit_for_actor(victim_id, opponent)
        });

        // Compute per-strike startup frames from the victim's sprite.
        let victim_sprite_frames: Option<[i16; crate::weapons::NUM_NORMAL_SWORD_STRIKES]> = self
            .get_entity(victim_id)
            .map(|e| &e.element_data().sprite)
            .map(|sprite| {
                use crate::combat::NORMAL_STRIKES;
                let mut frames = [0i16; crate::weapons::NUM_NORMAL_SWORD_STRIKES];
                for (i, &s) in NORMAL_STRIKES.iter().enumerate() {
                    let anim = strike_to_animation(s);
                    frames[i] = sprite.frames_from_start_till_action_done(anim) as i16;
                }
                frames
            });
        if victim_sprite_frames.is_none() {
            tracing::warn!(
                ?victim_id,
                "ConsiderToBeginParade: no sprite data for victim, using estimated strike startup frames"
            );
        }

        // Parry startup frames from victim's sprite.
        let parry_startup: Option<i16> = self
            .get_entity(victim_id)
            .map(|e| &e.element_data().sprite)
            .map(|sprite| {
                sprite.frames_from_start_till_action_done(
                    crate::order::OrderType::TransitionWaitingSwordParryingSword,
                ) as i16
            });

        let strike_ctx = crate::combat::StrikeSelectionContext {
            attacker_profile: victim_profile,
            fighting_ability: victim_fighting_ability,
            blood_alcohol: victim_blood_alcohol,
            is_rank_soldier: victim_is_rank_soldier,
            attacker_direction: victim_direction,
            attacker_elevation: victim_elevation,
            attacker_camp: victim_camp,
            // A victim caught by a circle strike while not duelling has an
            // empty opponent list; the proposal bails out for it.
            is_swordfighting: principal_opponent.is_some(),
            opponent_time_limit,
            strike_startup_frames: victim_sprite_frames,
            parry_startup_frames: parry_startup,
            is_npc: true,
        };

        let rng_before = debug.and_then(|_| self.control.rng.original_replay_cursor());
        let mut sweep_rebase = None;
        let proposed = crate::combat::propose_good_sword_strike_with_debug(
            sim,
            &strike_ctx,
            &nearby,
            &mut victim_boredom,
            true, // also_parade — this is the reactive parry path
            debug,
            &mut sweep_rebase,
        );
        self.apply_strike_selection_sweep_rebase(assets, victim_id, sweep_rebase);
        if let Some(debug) = debug {
            eprintln!(
                "[REACTIVE_SWORD frame={} co={} victim={} attacker={} phase=proposal_boundary caller=reactive_warning rng_before={:?} rng_after={:?} result={:?}]",
                debug.frame,
                debug.victim_creation_order,
                debug.victim,
                debug.attacker,
                rng_before,
                self.control.rng.original_replay_cursor(),
                proposed,
            );
        }

        // Write back boredom state
        if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(victim_id) {
            s.human.sword_strike_boredom = victim_boredom;
        }

        // ── 5. Handle the proposed action ────────────────────────────
        match proposed {
            Some(crate::combat::ProposedCombatAction::Parry) => {
                const MIN_CAPACITY_AVOID_PUSH_BACK: u16 = 50;

                let step_back_debug = step_back_debug.filter(|debug| {
                    debug.frame == self.control.frame_counter
                        && debug.creation_order == self.world.original_creation_order(victim_id)
                });
                if let Some(debug) = step_back_debug {
                    let victim = self.world.entities.get(victim_id).unwrap_or_else(|| {
                        panic!("reactive step-back diagnostic victim {victim_id:?} disappeared")
                    });
                    let ai = victim.ai_controller().unwrap_or_else(|| {
                        panic!("reactive step-back diagnostic victim {victim_id:?} lost AI")
                    });
                    eprintln!(
                        "REACTIVE_STEP_BACK frame={} co={} phase=parry_selected victim={} attacker={} fighting_ability={} push_back_distance={} state={:?} substate={:?} position=({:08x},{:08x},sector={:?},level={}) animation={:?} command={:?} couldnt={} already={} inside_think={} owner_work={:?}",
                        debug.frame,
                        debug.creation_order,
                        victim_id.index(),
                        attacker_id.index(),
                        victim_fighting_ability,
                        push_back_distance,
                        ai.current_state,
                        ai.current_substate,
                        victim.element_data().position_map().x.to_bits(),
                        victim.element_data().position_map().y.to_bits(),
                        victim.element_data().sector(),
                        victim.element_data().layer(),
                        victim
                            .actor_data()
                            .and_then(|actor| actor.installed_order)
                            .map(|order| order.order_type)
                            .unwrap_or(crate::order::OrderType::NonanimationEnd),
                        self.actor_command(victim_id),
                        ai.couldnt_reachpoint,
                        ai.already_on_point,
                        ai.completion_latch_inside_think,
                        ai.outbox.reentrant.owner_work,
                    );
                }

                // StopAll interrupts the selected element, but Original does
                // not install an idle sprite order before this method's
                // following GoTo reads GetAnimation/GetActionState. Rust's
                // halt cleanup normalizes that actor state eagerly, so retain
                // the complete live owner context across the narrow barrier.
                let mut step_back_ctx = if victim_fighting_ability >= MIN_CAPACITY_AVOID_PUSH_BACK
                    && push_back_distance != 0
                {
                    let scratch = self.build_owner_context_scratch_without_forecast(assets);
                    let victim_sector = self
                        .world
                        .entities
                        .get(victim_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "ConsiderToBeginParade step-back victim {} disappeared",
                                victim_id.index()
                            )
                        })
                        .element_data()
                        .sector();
                    let building_sector = self.entity_building_sector(victim_sector);
                    let mut ctx = {
                        let victim = self.world.entities.get(victim_id).unwrap_or_else(|| {
                            panic!(
                                "ConsiderToBeginParade step-back victim {} disappeared",
                                victim_id.index()
                            )
                        });
                        crate::engine::ai::build_ai_context_from_entity(
                            victim,
                            self.control.frame_counter,
                            building_sector,
                            self.world.weather.is_forest_level,
                            self.world.weather.ambiance,
                            self.ai.standard_view_polygon_radius,
                            &scratch.ai_entity_views,
                            &scratch.ai_sight_obstacles,
                            &self.world.fast_grid,
                            &assets.hiking_paths,
                            &assets.hiking_waypoint_sectors,
                            &self.ai.global.all_soldier_handles,
                            self.control.sim_config.difficulty,
                        )
                    };
                    self.refresh_selected_default_wait_identity(victim_id, &mut ctx);
                    Some(ctx)
                } else {
                    None
                };

                // StopAll().
                if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(victim_id)
                    && let Some(ai) = s.npc.ai_brain.base_mut()
                {
                    ai.stop_all();
                    // StopMovement only preserves the three ordinary
                    // upright/crouched locomotion actions. Sword movement and
                    // every other interruptible order are cleared, so the
                    // immediately following Original GoTo observes
                    // RHNONANIMATION_END rather than the pre-Stop animation.
                    // Rust installs its fallback Wait eagerly during the halt
                    // drain, so project that narrow null-order boundary onto
                    // the retained call-site context before using it below.
                    if let Some(ctx) = step_back_ctx.as_mut()
                        && ai.pending_halt_exposes_goto_idle(ctx)
                    {
                        ctx.self_animation = crate::order::OrderType::NonanimationEnd;
                    }
                }
                // `RHArtificialMalignity::ConsiderToBeginParade` performs
                // StopAll synchronously before either GoTo or the parry
                // sequence launch. Close the callback boundary and then apply
                // that narrow halt barrier now, so it cannot interrupt the
                // replacement work below.
                self.drain_ai_owner_halt_boundary(sim, assets, victim_id);

                // Step-back dodge for push-back strikes if
                // fighting ability is high enough.
                if victim_fighting_ability >= MIN_CAPACITY_AVOID_PUSH_BACK
                    && push_back_distance != 0
                {
                    let attacker_pos_map = self
                        .get_entity(attacker_id)
                        .map(|e| e.element_data().position_map())
                        .unwrap_or(victim_pos);
                    let (victim_sector, victim_move_box) = self
                        .get_entity(victim_id)
                        .map(|e| {
                            let sector = e.element_data().sector();
                            let mbox = if e.actor_data().is_some() {
                                *e.position_iface().get_move_box()
                            } else {
                                Default::default()
                            };
                            (sector, mbox)
                        })
                        .unwrap_or((None, Default::default()));
                    let victim_ai_pos = crate::ai::Position {
                        x: victim_pos.x,
                        y: victim_pos.y,
                        sector: victim_sector,
                        level: victim_layer,
                    };
                    let attacker_ai_pos = crate::ai::Position {
                        x: attacker_pos_map.x,
                        y: attacker_pos_map.y,
                        sector: None,
                        level: victim_layer,
                    };
                    let good_dist = push_back_distance + 20;
                    let min_dist = push_back_distance + 10;
                    // The push-back geometry is resolved in
                    // un-isometric sword-fight space, so pass
                    // `SWORDFIGHT_ASPECT_RATIO` (= 1.0) instead of
                    // the default `ASPECT_RATIO` (0.5735).
                    let step_back_goal = crate::ai_enemy::propose_good_step_back_goal(
                        victim_ai_pos,
                        &victim_move_box,
                        attacker_ai_pos,
                        good_dist,
                        min_dist,
                        Some(&self.world.fast_grid),
                        crate::position_interface::SWORDFIGHT_ASPECT_RATIO,
                    );
                    if let Some(debug) = step_back_debug {
                        eprintln!(
                            "REACTIVE_STEP_BACK frame={} co={} phase=goal_result victim={} attacker={} victim_position=({:08x},{:08x},sector={:?},level={}) attacker_position=({:08x},{:08x}) good_distance={} min_distance={} result={:?}",
                            debug.frame,
                            debug.creation_order,
                            victim_id.index(),
                            attacker_id.index(),
                            victim_ai_pos.x.to_bits(),
                            victim_ai_pos.y.to_bits(),
                            victim_ai_pos.sector,
                            victim_ai_pos.level,
                            attacker_ai_pos.x.to_bits(),
                            attacker_ai_pos.y.to_bits(),
                            good_dist,
                            min_dist,
                            step_back_goal,
                        );
                    }
                    if let Some(step_back_goal) = step_back_goal {
                        let ctx = step_back_ctx.take().unwrap_or_else(|| {
                            panic!(
                                "ConsiderToBeginParade step-back victim {} has no retained GoTo context",
                                victim_id.index()
                            )
                        });

                        // Step back to avoid strike.
                        if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(victim_id)
                            && let crate::element::AiBrain::Enemy(ref mut ai) = s.npc.ai_brain
                        {
                            let flags = if ctx.self_is_rider {
                                crate::ai::GotoFlags::SWORD
                            } else {
                                crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::SWORD
                            };
                            ai.go_to(
                                crate::ai::AiState::Attacking,
                                crate::ai::Substate::AttackingSwordfightStepBack,
                                step_back_goal,
                                flags,
                                &ctx,
                            );
                            if let Some(debug) = step_back_debug {
                                eprintln!(
                                    "REACTIVE_STEP_BACK frame={} co={} phase=after_goto victim={} state={:?} substate={:?} goal=({:08x},{:08x},sector={:?},level={}) flags={:?} couldnt={} already={} inside_think={} pending_orders={} owner_work={:?}",
                                    debug.frame,
                                    debug.creation_order,
                                    victim_id.index(),
                                    ai.base.current_state,
                                    ai.base.current_substate,
                                    step_back_goal.x.to_bits(),
                                    step_back_goal.y.to_bits(),
                                    step_back_goal.sector,
                                    step_back_goal.level,
                                    flags,
                                    ai.base.couldnt_reachpoint,
                                    ai.base.already_on_point,
                                    ai.base.completion_latch_inside_think,
                                    ai.base.outbox.actor.orders.len(),
                                    ai.base.outbox.reentrant.owner_work,
                                );
                            }
                        }
                        // This branch returns immediately after GoTo; close the
                        // owner-local callback boundary before the caller resumes.
                        self.drain_direct_ai_owner_boundary_without_forecast(
                            sim, victim_id, assets,
                        );
                        if let Some(debug) = step_back_debug {
                            let victim = self.world.entities.get(victim_id).unwrap_or_else(|| {
                                panic!(
                                    "reactive step-back diagnostic victim {victim_id:?} disappeared after drain"
                                )
                            });
                            let ai = victim.ai_controller().unwrap_or_else(|| {
                                panic!(
                                    "reactive step-back diagnostic victim {victim_id:?} lost AI after drain"
                                )
                            });
                            eprintln!(
                                "REACTIVE_STEP_BACK frame={} co={} phase=after_drain victim={} state={:?} substate={:?} animation={:?} command={:?} couldnt={} already={} inside_think={} self_stimuli={:?} owner_work={:?}",
                                debug.frame,
                                debug.creation_order,
                                victim_id.index(),
                                ai.current_state,
                                ai.current_substate,
                                victim
                                    .actor_data()
                                    .and_then(|actor| actor.installed_order)
                                    .map(|order| order.order_type)
                                    .unwrap_or(crate::order::OrderType::NonanimationEnd),
                                self.actor_command(victim_id),
                                ai.couldnt_reachpoint,
                                ai.already_on_point,
                                ai.completion_latch_inside_think,
                                ai.outbox.reentrant.self_stimuli,
                                ai.outbox.reentrant.owner_work,
                            );
                        }
                        tracing::debug!(
                            ?victim_id,
                            ?attacker_id,
                            ?step_back_goal,
                            "ConsiderToBeginParade: step-back dodge"
                        );
                        return;
                    }
                }

                // Normal parade.  Launch parry sequence element.
                let mut seq = crate::sequence::Sequence::new();
                let parry_elem =
                    crate::sequence::SequenceElement::new(1, Command::ParrySword, Some(victim_id));
                seq.append_element(parry_elem);
                self.launch_sequence(seq);

                // Timer: attacker's strike duration + 10-frame
                // buffer.  Hoist the sprite read before the mutable
                // borrow below.
                let attacker_anim_frames: u16 = match self
                    .get_entity(attacker_id)
                    .map(|e| &e.element_data().sprite)
                    .map(|sprite| {
                        sprite.frames_from_start_till_action_done(strike_to_animation(
                            animation_strike,
                        ))
                    }) {
                    Some(f) => f,
                    None => {
                        tracing::warn!(
                            ?attacker_id,
                            ?animation_strike,
                            "ConsiderToBeginParade: no sprite data for attacker, using estimated strike frames for parade timer"
                        );
                        crate::combat::STRIKE_STARTUP_FRAMES
                            .get(animation_strike as usize)
                            .copied()
                            .unwrap_or(25) as u16
                    }
                };
                let strike_frames = attacker_anim_frames as u32 + 10;

                // Opt-in trace for the reactive-parade heartbeat timer.
                //
                // `SUBSTATE_ATTACKING_SWORDFIGHT_PARADE` is only left again on
                // `EVENT_TIMER`, so this single duration decides how long a
                // parrying soldier stays out of the ordinary
                // `ReconsiderSwordfight` heartbeat. When a parity frontier
                // shows one fighter running (or skipping) a reconsideration
                // group, this dumps every operand of
                // `RHSprite::GetFramesFromStartTillActionDone`
                // (`original-code/RHsprite.cpp:1283`) for the attacker so the
                // ring frame can be reconstructed by hand.
                if std::env::var_os("PARITY_DEBUG_PARADE_TIMER").is_some() {
                    let anim = strike_to_animation(animation_strike);
                    let sprite = &self
                        .get_entity(attacker_id)
                        .unwrap_or_else(|| {
                            panic!("parade timer diagnostic attacker {attacker_id:?} disappeared")
                        })
                        .element_data()
                        .sprite;
                    let row = sprite.current_conversion()[anim as usize];
                    let waits = |r: u16| {
                        (0..sprite.num_frames_for_row(r))
                            .map(|i| sprite.wait_time(r, i))
                            .collect::<Vec<_>>()
                    };
                    eprintln!(
                        "[PARADE_TIMER] frame={} victim_co={:?} attacker_co={:?} animation={anim:?} conversion_row={row} action_done={} num_frames={} waits={:?} live_row={} live_frame={} frames={attacker_anim_frames} ring_in={strike_frames}",
                        self.control.frame_counter,
                        self.world.original_creation_order(victim_id),
                        self.world.original_creation_order(attacker_id),
                        sprite.action_done_for_row(row),
                        sprite.num_frames_for_row(row),
                        waits(row),
                        sprite.current_row,
                        sprite.current_frame,
                    );
                }

                // Set substate to parade
                if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(victim_id)
                    && let crate::element::AiBrain::Enemy(ref mut ai) = s.npc.ai_brain
                {
                    ai.set_state(
                        crate::ai::AiState::Attacking,
                        crate::ai::Substate::AttackingSwordfightParade,
                    );
                }
                self.drain_ai_owner_work_for(sim, assets, victim_id);
                if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(victim_id)
                    && let crate::element::AiBrain::Enemy(ref mut ai) = s.npc.ai_brain
                {
                    ai.base
                        .launch_timer(strike_frames, self.control.frame_counter);
                }

                tracing::debug!(
                    ?victim_id,
                    ?attacker_id,
                    ?animation_strike,
                    "ConsiderToBeginParade: parrying"
                );
            }

            Some(crate::combat::ProposedCombatAction::Strike(counter_strike)) => {
                // Counter-strike.  Order:
                //   MarkForSpecialStrike → SetState → StopAll →
                //   Launch.
                // MarkForSpecialStrike sets the X-mark emoticon and
                // routes the state transition through
                // `begin_special_strike` (single owner for the
                // transition); StopAll is applied immediately before
                // the counter-strike sequence is queued.
                if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(victim_id)
                    && let crate::element::AiBrain::Enemy(ref mut ai) = s.npc.ai_brain
                {
                    ai.base.set_emoticon(crate::ai::EmoticonType::XMark);
                    ai.begin_special_strike();
                    ai.base.stop_all();
                }
                self.drain_ai_owner_halt_boundary(sim, assets, victim_id);

                // Launch counter-strike sequence
                let counter_cmd = counter_strike.to_command();
                // The counter always goes to the principal opponent. A
                // victim with an empty opponent list cannot reach this arm
                // in the Original — the proposal refuses outright for an
                // actor that is not swordfighting — so there is no
                // substitute target to fall back to here.
                let Some(target) = principal_opponent else {
                    tracing::warn!(
                        ?victim_id,
                        ?attacker_id,
                        ?counter_strike,
                        "ConsiderToBeginParade: counter-strike proposed for a victim with no principal opponent; dropping it"
                    );
                    return;
                };

                let mut seq = crate::sequence::Sequence::new();
                let strike_elem = crate::sequence::SequenceElement::new_interaction(
                    1,
                    counter_cmd,
                    Some(victim_id),
                    Some(target),
                );
                seq.append_element(strike_elem);
                self.launch_sequence(seq);

                tracing::debug!(
                    ?victim_id,
                    ?attacker_id,
                    ?counter_strike,
                    "ConsiderToBeginParade: counter-strike"
                );
            }

            None => {
                // Do nothing.
            }
        }
    }

    /// Soldier AI learning: record a sword strike that hit them so they
    /// can avoid it in the future.
    ///
    /// For circular hits (H, I), dispatches the experience to all
    /// nearby soldier friends with sufficient IQ.
    pub(in crate::engine) fn make_bad_sword_strike_experience(
        &mut self,
        assets: &LevelAssets,
        soldier_id: EntityId,
        strike: SwordStrike,
        dispatch_to_all: bool,
    ) {
        const MIN_CAPACITY_TO_MEMORIZE_TWO_STRIKES: u16 = 50;
        const MIN_CAPACITY_TO_MEMORIZE_THREE_STRIKES: u16 = 80;
        const MIN_CAPACITY_LEARNING_BY_LOOKING: u16 = 70;
        const MAX_DISTANCE_LEARNING_BY_LOOKING: f32 = 400.0;

        let is_circular = matches!(strike, SwordStrike::H | SwordStrike::I);
        // Only real strikes get memorized — others are domino effects
        if !matches!(
            strike,
            SwordStrike::A
                | SwordStrike::B
                | SwordStrike::C
                | SwordStrike::D
                | SwordStrike::E
                | SwordStrike::F
                | SwordStrike::G
                | SwordStrike::H
                | SwordStrike::I
        ) {
            return;
        }

        // Dispatch to nearby friendly soldiers if this was a circular hit
        if dispatch_to_all && is_circular {
            let (camp, my_pos) = {
                let entity = match self.get_entity(soldier_id) {
                    Some(e) => e,
                    None => return,
                };
                match entity {
                    Entity::Soldier(s) => {
                        (s.soldier.cached_camp, entity.element_data().position_map())
                    }
                    _ => return,
                }
            };

            let friend_ids: Vec<EntityId> = self
                .world
                .entities
                .npc_ids()
                .filter(|&id| id != soldier_id)
                .filter(|&id| {
                    let Some(entity @ Entity::Soldier(s)) = self.world.entities.get(id) else {
                        return false;
                    };
                    if s.soldier.cached_camp != camp {
                        return false;
                    }
                    // AI state Attacking
                    if s.npc.ai_state() != crate::ai::AiState::Attacking {
                        return false;
                    }
                    // Skill check. Original calls the virtual
                    // `GetFightingAbility()`, which applies the active
                    // difficulty modifier for Lacklandist soldiers — not the
                    // raw profile capacity.
                    let friend_ability = fighting_ability_from_profile(
                        entity,
                        &assets.profile_manager,
                        self.control.sim_config.difficulty,
                    );
                    if friend_ability < MIN_CAPACITY_LEARNING_BY_LOOKING {
                        return false;
                    }
                    // Distance check (bounding box). Original centers the box
                    // on the learner's `GetPositionMap()` but tests the
                    // friend's `GetPositionGround()` (world x/y, i.e. map y
                    // plus elevation).
                    let fpos = s.element.position();
                    (fpos.x - my_pos.x).abs() <= MAX_DISTANCE_LEARNING_BY_LOOKING
                        && (fpos.y - my_pos.y).abs() <= MAX_DISTANCE_LEARNING_BY_LOOKING
                })
                .collect();

            for friend_id in friend_ids {
                self.make_bad_sword_strike_experience(assets, friend_id, strike, false);
            }
        }

        // Update this soldier's memory
        let strike_opt = Some(strike);
        let fighting_ability = self
            .get_entity(soldier_id)
            .map(|e| {
                fighting_ability_from_profile(
                    e,
                    &assets.profile_manager,
                    self.control.sim_config.difficulty,
                )
            })
            .unwrap_or(0);

        let bad_experience_debug = std::env::var_os("PARITY_DEBUG_BAD_EXPERIENCE").is_some();
        if bad_experience_debug {
            let creation_order = self.world.original_creation_order(soldier_id);
            eprintln!(
                "[BAD_EXPERIENCE frame={} co={} soldier={} strike={:?} ability={}]",
                self.control.frame_counter,
                creation_order,
                soldier_id.index(),
                strike,
                fighting_ability,
            );
        }
        if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(soldier_id)
            && let crate::element::AiBrain::Enemy(ref mut ai) = s.npc.ai_brain
        {
            if ai.known_enemy_strike_1 == strike_opt {
                // Already memorized as last strike
                return;
            }
            if ai.known_enemy_strike_2 == strike_opt {
                // Swap 1st and 2nd
                ai.known_enemy_strike_2 = ai.known_enemy_strike_1;
                ai.known_enemy_strike_1 = strike_opt;
            } else {
                // Push onto head of list
                ai.known_enemy_strike_3 = ai.known_enemy_strike_2;
                ai.known_enemy_strike_2 = ai.known_enemy_strike_1;
                ai.known_enemy_strike_1 = strike_opt;
            }

            // Forget based on IQ
            if fighting_ability < MIN_CAPACITY_TO_MEMORIZE_TWO_STRIKES {
                ai.known_enemy_strike_2 = None;
            }
            if fighting_ability < MIN_CAPACITY_TO_MEMORIZE_THREE_STRIKES {
                ai.known_enemy_strike_3 = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_within_smalltalk_strike_range, opponent_sword_strike_time_limit,
        resolve_stale_impossible_action_done,
    };
    use crate::combat::{self, NearbyVictim, ProposedCombatAction, StrikeSelectionContext};
    use crate::element::Camp;
    use crate::profiles::{HtHWeaponProfile, ThrustProfile, WeaponThrustKind};
    use crate::weapons::SwordStrike;

    #[test]
    fn smalltalk_strike_range_truncates_squared_distance_like_original() {
        assert!(is_within_smalltalk_strike_range(10, 100.0));
        assert!(is_within_smalltalk_strike_range(10, 100.999));
        assert!(!is_within_smalltalk_strike_range(10, 101.0));
    }

    fn reactive_pc_proposal(opponent_animation: crate::order::OrderType) -> ProposedCombatAction {
        let mut profile = HtHWeaponProfile::default();
        profile.thrusts[SwordStrike::A as usize] = ThrustProfile {
            kind: WeaponThrustKind::Straight,
            cutting: 10,
            maximal_distance: 100,
            ..Default::default()
        };
        let deadline = opponent_sword_strike_time_limit(Some(opponent_animation), || {
            crate::sprite::ActionDoneTiming::Frames(5)
        });
        let ctx = StrikeSelectionContext {
            attacker_profile: &profile,
            fighting_ability: 100,
            blood_alcohol: 0,
            is_rank_soldier: false,
            attacker_direction: 0,
            attacker_elevation: 0.0,
            attacker_camp: Camp::Royalists,
            is_swordfighting: true,
            opponent_time_limit: Some(deadline),
            strike_startup_frames: Some([10; crate::weapons::NUM_NORMAL_SWORD_STRIKES]),
            parry_startup_frames: Some(1),
            is_npc: false,
        };
        let victim = NearbyVictim {
            eligible_for_regular_strikes: true,
            dx: 0.0,
            dy_stretched: -20.0,
            distance: 20.0,
            direction_sector: 0,
            camp: Camp::Lacklandists,
            facing_direction: 8,
            elevation: 0.0,
            life_points: 100,
            defender_profile: None,
            is_primary_target: true,
            is_walking_with_sword: false,
        };

        combat::propose_good_sword_strike(
            &crate::sim_rng::test_context(),
            &ctx,
            &[victim],
            &mut vec![0; crate::weapons::NUM_NORMAL_SWORD_STRIKES],
            true,
        )
        .expect("high-skill reactive PC should choose a strike or parry")
    }

    #[test]
    fn executing_sword_deadline_forces_reactive_pc_to_parry() {
        assert_eq!(
            reactive_pc_proposal(crate::order::OrderType::ExecutingSword),
            ProposedCombatAction::Parry
        );
    }

    #[test]
    fn striking_down_sword_keeps_reactive_pc_counterstrike_unlimited() {
        assert_eq!(
            reactive_pc_proposal(crate::order::OrderType::StrikingDownSword),
            ProposedCombatAction::Strike(SwordStrike::A)
        );
    }

    #[test]
    fn s075_impossible_action_done_rejects_frame_731_then_valid_deadline_accepts_frame_732() {
        let profile = HtHWeaponProfile::default();
        let propose = |timing| {
            let deadline = opponent_sword_strike_time_limit(
                Some(crate::order::OrderType::StrikingStraightSword),
                || timing,
            );
            let ctx = StrikeSelectionContext {
                attacker_profile: &profile,
                fighting_ability: 100,
                blood_alcohol: 0,
                is_rank_soldier: false,
                attacker_direction: 0,
                attacker_elevation: 0.0,
                attacker_camp: Camp::Royalists,
                is_swordfighting: true,
                opponent_time_limit: Some(deadline),
                strike_startup_frames: Some([10; crate::weapons::NUM_NORMAL_SWORD_STRIKES]),
                parry_startup_frames: Some(0),
                is_npc: false,
            };
            combat::propose_good_sword_strike(
                &crate::sim_rng::test_context(),
                &ctx,
                &[],
                &mut vec![0; crate::weapons::NUM_NORMAL_SWORD_STRIKES],
                true,
            )
        };

        assert_eq!(
            propose(crate::sprite::ActionDoneTiming::Impossible),
            None,
            "frame 731's impossible action-done marker must not become the permissive -1 fallback"
        );
        assert_eq!(
            propose(crate::sprite::ActionDoneTiming::Frames(18)),
            Some(ProposedCombatAction::Parry),
            "frame 732's valid 18-frame deadline admits the zero-frame parry transition"
        );
    }

    #[test]
    fn historical_trace_keeps_uncaptured_stale_impossible_deadline_rejecting() {
        assert_eq!(
            resolve_stale_impossible_action_done(None),
            crate::sprite::ActionDoneTiming::Impossible,
            "an older trace without the captured over-read must not turn an impossible marker into permissive -1",
        );
        assert_eq!(
            resolve_stale_impossible_action_done(Some(-16230)),
            crate::sprite::ActionDoneTiming::Frames(-16230),
            "newer traces preserve the captured legacy wrapped deadline",
        );
    }

    #[test]
    fn enemy_executing_sword_deadline_rejects_b_and_selects_a() {
        let mut profile = HtHWeaponProfile::default();
        profile.thrusts[SwordStrike::A as usize] = ThrustProfile {
            kind: WeaponThrustKind::Straight,
            cutting: 10,
            maximal_distance: 100,
            ..Default::default()
        };
        profile.thrusts[SwordStrike::B as usize] = ThrustProfile {
            kind: WeaponThrustKind::Straight,
            cutting: 20,
            maximal_distance: 100,
            ..Default::default()
        };
        let victim = NearbyVictim {
            eligible_for_regular_strikes: true,
            dx: 0.0,
            dy_stretched: -20.0,
            distance: 20.0,
            direction_sector: 0,
            camp: Camp::Lacklandists,
            facing_direction: 8,
            elevation: 0.0,
            life_points: 100,
            defender_profile: None,
            is_primary_target: true,
            is_walking_with_sword: false,
        };
        let propose = |opponent_animation| {
            let deadline = opponent_sword_strike_time_limit(Some(opponent_animation), || {
                crate::sprite::ActionDoneTiming::Frames(10)
            });
            let ctx = StrikeSelectionContext {
                attacker_profile: &profile,
                fighting_ability: 100,
                blood_alcohol: 0,
                is_rank_soldier: false,
                attacker_direction: 0,
                attacker_elevation: 0.0,
                attacker_camp: Camp::Royalists,
                is_swordfighting: true,
                opponent_time_limit: Some(deadline),
                strike_startup_frames: Some([7, 12, 25, 7, 7, 8, 8, 8, 8]),
                parry_startup_frames: None,
                is_npc: true,
            };
            combat::propose_good_sword_strike(
                &crate::sim_rng::test_context(),
                &ctx,
                std::slice::from_ref(&victim),
                &mut vec![0; crate::weapons::NUM_NORMAL_SWORD_STRIKES],
                false,
            )
        };

        assert_eq!(
            propose(crate::order::OrderType::ExecutingSword),
            Some(ProposedCombatAction::Strike(SwordStrike::A)),
            "ExecutingSword is strike C in Original, so raw deadline 10 admits A startup 7 but rejects B startup 12"
        );
        assert_eq!(
            propose(crate::order::OrderType::StrikingDownSword),
            Some(ProposedCombatAction::Strike(SwordStrike::B)),
            "StrikingDownSword is not an Original A-I strike and keeps the unlimited control deadline"
        );
    }
}
