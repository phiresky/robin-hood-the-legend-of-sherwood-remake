//! Swordfight evaluation, parade decisions, propose/launch strikes.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;
use crate::combat::{self};
use crate::element::{ActionState, Command, Entity, EntityId};
use crate::profiles::WeaponThrustKind;
use crate::weapons::SwordStrike;

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
            Some(h) => h.opponents.clone(),
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
            let opp_jump_line = human.opponent_jump_lines.first().copied().flatten();

            // Selected PC short-circuits.
            if entity.is_pc() && self.selected_pc_ids().contains(&entity_id) {
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
        if geo_distance > my_max_range + SWORDFIGHT_DISTANCE_EPSILON
            && geo_distance > opp_max_range + SWORDFIGHT_DISTANCE_EPSILON
            && !last_step_back
        {
            geo_movement = backward_dist;
        }
        // Too close → walk back with force-movement.
        else if geo_distance + SWORDFIGHT_DISTANCE_EPSILON < my_min_range {
            geo_movement = -backward_dist;
            force_movement = true;
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
        self.evaluate_swordfight_for(sim, assets, entity_id);
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
        if elevation_prune || range_or_los_prune {
            Self::remove_opponent(&mut self.world.entities, entity_id, first_principal);
            Self::remove_opponent(&mut self.world.entities, first_principal, entity_id);
            self.recompute_relative_fighting_ability(entity_id, assets);
            self.recompute_relative_fighting_ability(first_principal, assets);
            self.evaluate_opponents(sim, assets, entity_id);
            self.evaluate_opponents(sim, assets, first_principal);
            return;
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
        } else if crate::sim_rng::u32(sim, crate::sim_rng::RngSite::MeleeNonMutualGate, 0..100)
            >= 10
        {
            return;
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
            .distance[crate::weapons::WeaponDistance::Maximal as usize]
                as f32;
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
        let near = self_max * self_max >= dx * dx + dy * dy + dz * dz;
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
            self.get_entity_mut(entity_id)
                .and_then(Entity::human_data_mut)
                .unwrap_or_else(|| {
                    panic!("EvaluateSwordfight step-back owner {entity_id:?} is not human")
                })
                .last_motion_was_step_back_in_combat = true;
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
        let already_striking = pc
            .actor_data()
            .unwrap_or_else(|| {
                panic!("EvaluateSwordfight strike proposal PC {pc_id:?} is not an actor")
            })
            .active_melee
            .is_active();
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
        let opponent_time_limit: Option<i16> = {
            let target = self.get_entity(target_id).unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight strike proposal PC {pc_id:?} references missing target {target_id:?}"
                )
            });
            target.actor_data().unwrap_or_else(|| {
                panic!(
                    "EvaluateSwordfight strike proposal PC {pc_id:?} target {target_id:?} is not an actor"
                )
            });
            let sprite = &target.element_data().sprite;
            use crate::order::OrderType as OT;
            let in_active_strike = matches!(
                self.live_actor_animation(target_id),
                Some(
                    OT::StrikingStraightSword
                        | OT::StrikingStraightStrongSword
                        | OT::StrikingRightSword
                        | OT::StrikingLeftSword
                        | OT::StrikingRoundRightSword
                        | OT::StrikingRoundLeftSword
                        | OT::StrikingSemiroundRightSword
                        | OT::StrikingSemiroundLeftSword
                        | OT::StrikingDownSword
                )
            );
            if !in_active_strike {
                Some(1000i16)
            } else {
                let ftad = sprite.frames_from_now_till_action_done();
                Some(if ftad == -1 { 1000 } else { ftad })
            }
        };

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
                let lp = match e {
                    Entity::Pc(pc) => pc.pc.life_points,
                    Entity::Soldier(s) => s.npc.life_points,
                    _ => 0,
                };
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
            is_swordfighting: true,
            opponent_time_limit,
            strike_startup_frames: attacker_sprite_frames,
            parry_startup_frames: parry_startup,
            is_npc: false,
        };

        let strike =
            match crate::combat::propose_good_sword_strike(sim, &ctx, &nearby, &mut boredom, false)
            {
                Some(crate::combat::ProposedCombatAction::Strike(s)) => s,
                _ => return,
            };

        // Persist the updated boredom array back onto the PC.
        if let Some(entity) = self.world.entities.get_mut(pc_id)
            && let Some(human) = entity.human_data_mut()
        {
            human.sword_strike_boredom = boredom;
        }

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
            .clone();
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
            .clone();
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
                let action = victim
                    .actor_data()
                    .map(|a| a.action_state)
                    .unwrap_or_default();
                (
                    is_selected_pc,
                    is_npc_soldier,
                    npc_substate,
                    ability,
                    action,
                )
            };
            let (is_selected_pc, is_npc_soldier, npc_substate, ability, action) = victim_info;

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
                // A soldier mid-special-strike can still parade an incoming
                // blow; Original explicitly includes that observable state.
                let in_swordfight_substate = matches!(
                    npc_substate,
                    Some(crate::ai::Substate::AttackingSwordfight)
                        | Some(crate::ai::Substate::AttackingSwordfightSpecialStrike)
                        | Some(crate::ai::Substate::AttackingMovingAroundOldEnemy)
                        | Some(crate::ai::Substate::AttackingApproachingNewEnemy)
                );
                if in_swordfight_substate {
                    self.consider_to_begin_parade(sim, assets, victim_id, attacker_id, strike);
                }
                continue;
            }

            // PC parade/counter-strike via
            // `propose_good_sword_strike(sim, also_parade=true)`, which
            // may produce a counter-strike or a parry fallback.
            //
            // Guard: `is_swordfighting() == false → return`.  That
            // covers every "sword drawn" action state — WaitingSword,
            // MovingSword, MovingFastSword, ParryingSword,
            // ParryingSwordLow — not just WaitingSword.
            if !action.is_sword() {
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
                let principal = match entity {
                    Entity::Pc(pc) => pc.pc.melee_target,
                    _ => None,
                };
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

            // Build nearby victims so circle/push/round strike scoring can
            // see adjacent enemies — same shape as the strike-launcher and
            // PC strike-propose paths.
            let inv_aspect = INVERSE_SWORDFIGHT_ASPECT_RATIO;
            let obstacles = crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.static_sight_obstacles.as_slice(),
                dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                static_active: &self.world.static_sight_obstacle_active,
            };
            let target_id_for_nearby = principal_opponent.unwrap_or(attacker_id);
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
                    let lp = match e {
                        Entity::Pc(pc) => pc.pc.life_points,
                        Entity::Soldier(s) => s.npc.life_points,
                        _ => 0,
                    };
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
                is_swordfighting: true,
                opponent_time_limit: None,
                strike_startup_frames: pc_sprite_frames,
                parry_startup_frames: pc_parry_startup,
                is_npc: false, // PC path — different skill gate
            };

            let proposed = crate::combat::propose_good_sword_strike(
                sim,
                &strike_ctx,
                &nearby,
                &mut pc_boredom,
                true, // also_parade
            );

            // Write back boredom
            if let Some(entity) = self.world.entities.get_mut(victim_id)
                && let Some(human) = entity.human_data_mut()
            {
                human.sword_strike_boredom = pc_boredom;
            }

            match proposed {
                Some(crate::combat::ProposedCombatAction::Parry) => {
                    // Launch a ParrySword sequence element.  Routing
                    // through the sequence manager preserves
                    // queue-level interrupt / bookkeeping that
                    // direct state mutation would skip.
                    let parry_elem = crate::sequence::SequenceElement::new(
                        1,
                        crate::element::Command::ParrySword,
                        Some(victim_id),
                    );
                    self.launch_element(parry_elem);
                }
                Some(crate::combat::ProposedCombatAction::Strike(counter_strike)) => {
                    // PC counter-strike: launch the strike sequence
                    // against the PC's duel partner — fall back to
                    // the incoming attacker when there's no current
                    // opponent.
                    let counter_cmd = counter_strike.to_command();
                    let target = principal_opponent.unwrap_or(attacker_id);
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
    pub(super) fn consider_to_begin_parade(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        attacker_id: EntityId,
        strike: SwordStrike,
    ) {
        // ── 1. Check if the victim recognizes this strike ────────────
        // Compare against the soldier's three known-enemy-strike slots.
        let strike_opt = Some(strike);
        let is_known = {
            let Some(Entity::Soldier(s)) = self.world.entities.get(victim_id) else {
                return;
            };
            let Some(ai) = s.npc.ai_brain.enemy() else {
                return;
            };
            strike_opt == ai.known_enemy_strike_1
                || strike_opt == ai.known_enemy_strike_2
                || strike_opt == ai.known_enemy_strike_3
        };
        if !is_known {
            return;
        }

        // ── 2. Record this strike experience (promote to head of list).
        self.make_bad_sword_strike_experience(assets, victim_id, strike, true);

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
                    let kind = profile.thrusts.get(strike as usize)?.kind;
                    match kind {
                        WeaponThrustKind::PushAside
                        | WeaponThrustKind::FalseCircle
                        | WeaponThrustKind::TrueCircle => {
                            Some(profile.thrusts[strike as usize].maximal_distance)
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
            let Some(Entity::Soldier(s)) = self.world.entities.get(victim_id) else {
                return;
            };
            let ai = match &s.npc.ai_brain {
                crate::element::AiBrain::Enemy(ai) => ai,
                _ => return,
            };
            let spi = s.soldier.soldier_profile_index;
            let sp = assets.profile_manager.get_soldier(spi);
            let fa = sp.map(|p| p.fighting).unwrap_or(50);
            let is_rank = sp
                .map(|p| p.rank == crate::profiles::ProfileRank::Soldier)
                .unwrap_or(true);
            let ba = ai.base.blood_alcohol;
            let camp = s.soldier.cached_camp;
            let pos = s.element.position_map();
            let elev = s.element.position().z;
            let layer = s.element.layer();
            let dir = s.element.direction();
            let boredom = s.human.sword_strike_boredom.clone();
            let principal = EntityId::Pc(crate::entity_id::PcId(ai.base.primary_target));
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
                let lp = match e {
                    Entity::Pc(pc) => pc.pc.life_points,
                    Entity::Soldier(s) => s.npc.life_points,
                    _ => 0,
                };
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
                    is_primary_target: eid == principal_opponent,
                    is_walking_with_sword,
                })
            })
            .collect();

        // ProposeGoodSwordStrike always times the defender's principal
        // opponent, which need not be the hitter that triggered this
        // reactive warning.
        let opponent_time_limit: Option<i16> = self.get_entity(principal_opponent).and_then(|e| {
            e.actor_data()?;
            let sprite = &e.element_data().sprite;
            use crate::order::OrderType as OT;
            let in_active_strike = matches!(
                self.live_actor_animation(principal_opponent),
                Some(
                    OT::StrikingStraightSword
                        | OT::StrikingStraightStrongSword
                        | OT::StrikingRightSword
                        | OT::StrikingLeftSword
                        | OT::StrikingRoundRightSword
                        | OT::StrikingRoundLeftSword
                        | OT::StrikingSemiroundRightSword
                        | OT::StrikingSemiroundLeftSword
                        | OT::StrikingDownSword
                )
            );
            if !in_active_strike {
                return Some(1000i16);
            }
            let ftad = sprite.frames_from_now_till_action_done();
            Some(if ftad == -1 { 1000 } else { ftad })
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
            is_swordfighting: true,
            opponent_time_limit,
            strike_startup_frames: victim_sprite_frames,
            parry_startup_frames: parry_startup,
            is_npc: true,
        };

        let proposed = crate::combat::propose_good_sword_strike(
            sim,
            &strike_ctx,
            &nearby,
            &mut victim_boredom,
            true, // also_parade — this is the reactive parry path
        );

        // Write back boredom state
        if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(victim_id) {
            s.human.sword_strike_boredom = victim_boredom;
        }

        // ── 5. Handle the proposed action ────────────────────────────
        match proposed {
            Some(crate::combat::ProposedCombatAction::Parry) => {
                const MIN_CAPACITY_AVOID_PUSH_BACK: u16 = 50;

                // StopAll().
                if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(victim_id)
                    && let Some(ai) = s.npc.ai_brain.base_mut()
                {
                    ai.stop_all();
                }
                // `RHArtificialMalignity::ConsiderToBeginParade` performs
                // StopAll synchronously before either GoTo or the parry
                // sequence launch. Close the callback boundary and then apply
                // that narrow halt barrier now, so it cannot interrupt the
                // replacement work below.
                self.drain_ai_owner_work_for(sim, assets, victim_id);
                self.apply_pending_ai_halt(victim_id);

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
                    if let Some(step_back_goal) = crate::ai_enemy::propose_good_step_back_goal(
                        victim_ai_pos,
                        &victim_move_box,
                        attacker_ai_pos,
                        good_dist,
                        min_dist,
                        Some(&self.world.fast_grid),
                        crate::position_interface::SWORDFIGHT_ASPECT_RATIO,
                    ) {
                        // Step back to avoid strike.
                        if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(victim_id)
                            && let crate::element::AiBrain::Enemy(ref mut ai) = s.npc.ai_brain
                        {
                            let flags = crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::SWORD;
                            let ctx = crate::ai::AiContext {
                                position: victim_ai_pos,
                                direction: victim_direction as u16,
                                ..Default::default()
                            };
                            ai.go_to(
                                crate::ai::AiState::Attacking,
                                crate::ai::Substate::AttackingSwordfightStepBack,
                                step_back_goal,
                                flags,
                                &ctx,
                            );
                        }
                        // This branch returns immediately after GoTo; close the
                        // owner-local callback boundary before the caller resumes.
                        self.drain_ai_owner_work_for(sim, assets, victim_id);
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
                        sprite.frames_from_start_till_action_done(strike_to_animation(strike))
                    }) {
                    Some(f) => f,
                    None => {
                        tracing::warn!(
                            ?attacker_id,
                            ?strike,
                            "ConsiderToBeginParade: no sprite data for attacker, using estimated strike frames for parade timer"
                        );
                        crate::combat::STRIKE_STARTUP_FRAMES
                            .get(strike as usize)
                            .copied()
                            .unwrap_or(25) as u16
                    }
                };
                let strike_frames = attacker_anim_frames as u32 + 10;

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
                    ?strike,
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
                }
                self.drain_ai_owner_work_for(sim, assets, victim_id);
                self.stop_owner(victim_id, crate::sequence::SequencePriority::Preference);

                // Launch counter-strike sequence
                let counter_cmd = counter_strike.to_command();
                let target = if principal_opponent.index() != 0 {
                    principal_opponent
                } else {
                    attacker_id
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
    pub(super) fn make_bad_sword_strike_experience(
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
                    let Some(Entity::Soldier(s)) = self.world.entities.get(id) else {
                        return false;
                    };
                    if s.soldier.cached_camp != camp {
                        return false;
                    }
                    // AI state Attacking
                    if s.npc.ai_state() != crate::ai::AiState::Attacking {
                        return false;
                    }
                    // IQ check
                    let friend_ability = assets
                        .profile_manager
                        .get_soldier(s.soldier.soldier_profile_index)
                        .map(|p| p.fighting)
                        .unwrap_or(0);
                    if friend_ability < MIN_CAPACITY_LEARNING_BY_LOOKING {
                        return false;
                    }
                    // Distance check (bounding box)
                    let fpos = s.element.position_map();
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
