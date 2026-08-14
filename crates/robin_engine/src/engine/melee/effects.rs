//! Push effects, falls, rolls, multi-target strikes.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;
use crate::combat::{self};
use crate::element::{ActionState, Entity, EntityId, EyeStatus, Posture};
use crate::profiles::WeaponThrustKind;
use crate::weapons::SwordStrike;

/// A lift sector's low entry point resolved to 3D: the lowest door's
/// `point_out` (map space), the plane-projected altitude of the low
/// sector's projection-area obstacle at that point, and the exit
/// layer / sector / obstacle used on landing.
#[derive(Debug)]
pub(super) struct LiftLowEntry {
    pub point: crate::coordinates::MapPoint,
    pub z: f32,
    pub layer: u16,
    pub sector: u16,
    pub obstacle: Option<u16>,
}

/// Compute the non-charge flight vector used by a falling-pushed order.
///
/// `RHElementActorHuman::ExecuteFallingPushed` accepts exactly these three
/// thrust kinds.  In all three cases the remaining strike distance is based
/// on the victim distance projected onto the attacker's direction, including
/// a negative projection when the victim is behind the attacker.
fn push_flight_vector(
    kind: WeaponThrustKind,
    attacker_dir: (f32, f32),
    attacker_pos: crate::coordinates::MapPoint,
    victim_pos: crate::coordinates::MapPoint,
    max_distance: f32,
) -> (f32, f32) {
    let dx = victim_pos.x - attacker_pos.x;
    let dy = victim_pos.y - attacker_pos.y;
    let projected_distance = dx * attacker_dir.0 + dy * attacker_dir.1;
    let remaining_distance = max_distance - projected_distance;

    match kind {
        WeaponThrustKind::PushAside => (
            attacker_dir.0 * remaining_distance,
            attacker_dir.1 * remaining_distance,
        ),
        WeaponThrustKind::TrueCircle | WeaponThrustKind::FalseCircle => {
            // SBGeoVector2D::Norm first rounds the squared sum as GEOTYPE
            // (f32), runs sqrt in double precision, then stores the result
            // back to GEOTYPE before Normalize divides each component.  The
            // platform `hypotf` sequence differs by an ULP for some live
            // combat vectors, which then shifts the retained flight goal.
            let squared_norm = dx * dx + dy * dy;
            let distance = f64::from(squared_norm).sqrt() as f32;
            if distance < 0.01 {
                (0.0, 0.0)
            } else {
                (
                    dx / distance * remaining_distance,
                    dy / distance * remaining_distance,
                )
            }
        }
        _ => panic!("unexpected push thrust kind: {kind:?}"),
    }
}

/// Select the animation queued by `TranslatePushDamage`.
///
/// The dead-rider arm precedes the posture switch in the original, so even a
/// rider whose current posture would normally suppress a pushed fall receives
/// `DyingUpright`.
fn translated_push_damage_animations(
    posture: Posture,
    action_state: ActionState,
    is_rider: bool,
    is_dead: bool,
) -> Option<PushDamageAnimations> {
    if is_rider && is_dead {
        Some(PushDamageAnimations {
            falling: crate::order::OrderType::DyingUpright,
            standing_up: None,
            stunned: None,
        })
    } else {
        select_push_damage_animations(posture, action_state)
    }
}

/// Immediate posture change performed by `TranslatePushDamage` itself.
///
/// Animated pushes leave posture untouched until the falling order reports
/// motion Start. Only the no-animation arm (already lying/dead/carried/etc.)
/// grounds a dead or unconscious victim during translation.
fn translated_push_posture(
    has_falling_animation: bool,
    is_dead: bool,
    is_unconscious: bool,
) -> Option<Posture> {
    if has_falling_animation {
        None
    } else if is_dead {
        Some(Posture::Dead)
    } else if is_unconscious {
        Some(Posture::Lying)
    } else {
        None
    }
}

/// Original `GetMoveBox(posture)` currently ignores its posture argument and
/// returns the actor's live primary movement box. `FindRollPoint` translates
/// that exact local box to the proposed destination before asking the grid to
/// adjust it to an authorized position.
fn roll_destination_box(
    move_box: crate::coordinates::MoveBox,
    destination: crate::coordinates::MapPoint,
) -> crate::coordinates::MapBBox {
    move_box.translated(destination)
}

/// Fatal push translation clears movement bookkeeping but leaves the actor's
/// live action state alone. Original `TranslatePushDamage` only authors the
/// falling order; `ExecuteFallingPushed` changes the state when that order
/// reaches the actor's Hourglass slot.
fn clear_fatal_push_path(actor: &mut crate::element::ActorData) {
    actor.clear_path();
}

impl EngineInner {
    /// Original `GetPossibleVictimsOfSwordStrike(..., true)` collector used by
    /// the MotionState::Start warning pass. This is intentionally separate
    /// from MOTION_DONE hit collection: in particular, lateral warnings admit
    /// every active human in the arc, while straight/assault warnings consider
    /// only the attacker's principal swordfight opponent.
    pub(super) fn collect_sword_strike_warning_victims(
        &self,
        assets: &LevelAssets,
        attacker_id: EntityId,
        strike: SwordStrike,
        profile_idx: Option<u32>,
    ) -> Vec<EntityId> {
        let profile_idx = profile_idx.unwrap_or_else(|| {
            panic!(
                "sword-strike warning collector attacker {attacker_id:?} has no HtH weapon profile id"
            )
        });
        let profile = assets
            .profile_manager
            .get_hth_weapon(profile_idx)
            .unwrap_or_else(|| {
                panic!(
                    "sword-strike warning collector attacker {attacker_id:?} has missing HtH weapon profile {profile_idx}"
                )
            });
        let Some(attacker) = self.get_entity(attacker_id) else {
            panic!("sword-strike warning collector lost attacker {attacker_id:?}");
        };
        let attacker_pos = attacker.element_data().position_map();
        let attacker_dir = attacker.element_data().direction();
        let thrust = &profile.thrusts[strike as usize];
        let min_dist = thrust.minimal_distance as f32;
        let max_dist = thrust.maximal_distance as f32;
        let obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };

        match thrust.kind {
            WeaponThrustKind::Straight | WeaponThrustKind::Assault => {
                let human = attacker.human_data().unwrap_or_else(|| {
                    panic!("sword-strike warning collector attacker {attacker_id:?} is not human")
                });
                let Some(principal_id) = human.opponents.first().copied() else {
                    return Vec::new();
                };
                let Some(principal) = self.get_entity(principal_id) else {
                    panic!(
                        "sword-strike warning collector attacker {attacker_id:?} has missing principal opponent {principal_id:?}"
                    );
                };
                let principal_pos = principal.element_data().position_map();
                let dx = attacker_pos.x - principal_pos.x;
                let dy = (attacker_pos.y - principal_pos.y) * INVERSE_SWORDFIGHT_ASPECT_RATIO;
                let distance = (dx * dx + dy * dy).sqrt();
                (distance >= min_dist && distance <= max_dist)
                    .then_some(principal_id)
                    .into_iter()
                    .collect()
            }
            WeaponThrustKind::Lateral => {
                let dir_angle = sector_to_angle(attacker_dir);
                let (begin_sector, end_sector) = match thrust.direction {
                    crate::profiles::WeaponThrustDirection::RightToLeft => (
                        angle_to_sector(dir_angle - strike_profile_angle(thrust.final_angle)),
                        angle_to_sector(dir_angle + strike_profile_angle(thrust.initial_angle)),
                    ),
                    _ => (
                        angle_to_sector(dir_angle - strike_profile_angle(thrust.initial_angle)),
                        angle_to_sector(dir_angle + strike_profile_angle(thrust.final_angle)),
                    ),
                };
                collect_lateral_warning_victims(
                    &self.world.entities,
                    attacker_id,
                    (attacker_pos.x, attacker_pos.y),
                    min_dist,
                    max_dist,
                    begin_sector,
                    end_sector,
                )
            }
            WeaponThrustKind::PushAside => collect_push_victims(
                &self.world.entities,
                &PushStrikeParams {
                    attacker_id,
                    attacker_pos: (attacker_pos.x, attacker_pos.y),
                    attacker_elevation: attacker.position_iface().get_elevation(),
                    position_space: PushStrikePositionSpace::Map,
                    attacker_direction: attacker_dir,
                    min_distance: min_dist,
                    max_distance: max_dist,
                    half_width: push_strike_half_width(thrust.repulsion),
                },
                &assets.profile_manager,
                &self.world.fast_grid,
                obstacles,
            ),
            WeaponThrustKind::TrueHalfCircle | WeaponThrustKind::FalseHalfCircle => {
                let dir_angle = sector_to_angle(attacker_dir);
                let (begin_sector, end_sector) = match thrust.direction {
                    crate::profiles::WeaponThrustDirection::RightToLeft => {
                        let initial = dir_angle + strike_profile_angle(thrust.initial_angle);
                        (
                            angle_to_sector(initial - std::f32::consts::PI),
                            angle_to_sector(initial),
                        )
                    }
                    _ => {
                        let initial = dir_angle - strike_profile_angle(thrust.initial_angle);
                        (
                            angle_to_sector(initial),
                            angle_to_sector(initial + std::f32::consts::PI),
                        )
                    }
                };
                collect_arc_victims(
                    &self.world.entities,
                    attacker_id,
                    (attacker_pos.x, attacker_pos.y),
                    min_dist,
                    max_dist,
                    begin_sector,
                    end_sector,
                    &assets.profile_manager,
                    &self.world.fast_grid,
                    obstacles,
                )
            }
            WeaponThrustKind::TrueCircle | WeaponThrustKind::FalseCircle => {
                collect_circle_warn_victims(
                    &self.world.entities,
                    attacker_id,
                    (attacker_pos.x, attacker_pos.y),
                    attacker_dir,
                    max_dist,
                    thrust.rotation_angle,
                    &assets.profile_manager,
                    &self.world.fast_grid,
                    obstacles,
                )
            }
        }
    }

    // ─── Push / stumble effects ─────────────────────────────────────

    /// Apply push-back movement and posture-aware falling animation to a
    /// victim from a push/circle/charge strike.
    ///
    /// Moves the victim away from the attacker, selects the correct
    /// falling-pushed animation based on posture/action state, and queues
    /// standup + stunned animations as needed.
    ///
    /// Returns `true` if the push handled the post-damage state transition
    /// (i.e. the caller should skip the regular hit reaction anim and
    /// `handle_post_damage`).
    /// Apply push effect with per-frame flight animation.
    ///
    /// Look up a lift sector's low-entry point via the cached
    /// `lowest_door_index` on the `GridSector`.
    ///
    /// Returns the lowest door's `point_out` (map space) together with
    /// the plane-projected altitude of the low sector's projection-area
    /// obstacle at that point (`z = 0` on flat ground), the exit layer,
    /// the exit sector, and the resolved obstacle index.  The door
    /// cache is populated at level load by
    /// `initialize_motion_from_level_data` (see the "Cache lowest /
    /// highest door per lift sector" pass), so the door lookup is O(1).
    pub(super) fn find_lift_low_entry(
        &self,
        assets: &LevelAssets,
        lift_sector: u16,
    ) -> Option<LiftLowEntry> {
        let grid_idx = *self
            .world
            .fast_grid
            .level
            .sector_number_map
            .get(&crate::sector::SectorNumber::new(lift_sector as i16))?;
        let gs = self.world.fast_grid.level.sectors.get(grid_idx)?;
        let door_idx = gs.lowest_door_index?;
        self.scripts.mission.as_ref()?;
        let door = self
            .script_domains
            .interactables
            .doors
            .get(door_idx as usize)?;
        let point = door.point_out;
        let layer = door.layer_out;
        let sector = u16::from(door.sector_out);
        let obstacle = self.get_projection_area_index(assets, sector, layer, point);
        let z = obstacle
            .and_then(|idx| {
                self.sight_obstacles(assets)
                    .get(idx as usize)
                    .map(|obs| obs.compute_top_z_from_projection(point.x, point.y))
            })
            .unwrap_or(0.0);
        Some(LiftLowEntry {
            point,
            z,
            layer,
            sector,
            obstacle,
        })
    }

    /// Translate a push applied to an entity on a ladder or wall:
    /// play the `FallingLadderWall` animation and flight the entity
    /// to the ladder's low entry point.
    pub(crate) fn translate_ladder_wall_fall(
        &mut self,
        assets: &LevelAssets,
        victim_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
    ) {
        let (victim_pos3, victim_sector) = match self.get_entity(victim_id) {
            Some(e) => (e.position_iface().get_position(), e.element_data().sector()),
            None => return,
        };

        // Destination is the ladder's low entry point, resolved to 3D
        // via the low sector's projection-area plane.  If we can't
        // locate it, leave the victim in place — the animation still
        // plays so the visual feedback is correct.
        let low_entry = victim_sector.and_then(|s| self.find_lift_low_entry(assets, u16::from(s)));

        // The sector should be a lift.  Log a warning if not rather
        // than crashing — we fall through to the safe path.
        if low_entry.is_none() {
            tracing::warn!(
                entity = ?victim_id,
                sector = ?victim_sector,
                "translate_ladder_wall_fall: no lowest door found for lift sector"
            );
        }

        // Free the lift occupancy so other actors can climb it.
        // Uses the `active_lift` marker that was set when the victim
        // entered the lift via WaitFreeLift.
        let active_lift = self
            .get_entity(victim_id)
            .and_then(|e| e.actor_data())
            .and_then(|a| a.active_lift);
        if let Some(lift) = active_lift {
            let victim_is_pc = self
                .get_entity(victim_id)
                .unwrap_or_else(|| {
                    panic!("active-lift victim {victim_id:?} vanished before forced release")
                })
                .is_pc();
            if let Some(grid_idx) = self
                .world
                .fast_grid
                .level
                .sector_number_map
                .get(&crate::sector::SectorNumber::new(lift.sector_number as i16))
                .copied()
            {
                let st = self.world.fast_grid_mut().lift_state_mut(grid_idx as u32);
                st.occupants = st.occupants.saturating_sub(1);
                if victim_is_pc {
                    st.occupants_pc = st.occupants_pc.saturating_sub(1);
                }
                if st.occupants == 0 {
                    st.occupied_upwards = false;
                    st.occupied_downwards = false;
                    st.wait_time = 0;
                }
            }
            // Clear the marker so the actor isn't credited with an
            // occupancy slot they no longer hold.
            if let Some(entity) = self.world.entities.get_mut(victim_id)
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.active_lift = None;
            }
        }

        // Insert FallingLadderWall onto the damage element.  Landing
        // (position snap, concussion, lying posture, order retirement)
        // is applied by the ladder-fall arm of `tick_push_flights` when
        // the tick countdown hits zero.
        self.queue_damage_anim(victim_id, damage_element, OrderType::FallingLadderWall);
        // Constant-speed fall toward the 3D low entry point: the
        // per-tick increment has fixed 3D length 10 and the flight
        // lasts `0.1 * distance` ticks.  A fall shorter than one step
        // installs no flight — the fall order then never arrives and
        // only ends when its sprite runs out, like the original.
        if let Some(entity) = self.world.entities.get_mut(victim_id)
            && let Some(entry) = &low_entry
        {
            // 3D vector from the victim's cached world position to the
            // low entry point (world y = map y + z).
            let dx = entry.point.x - victim_pos3.x;
            let dy3 = (entry.point.y + entry.z) - victim_pos3.y;
            let dz = entry.z - victim_pos3.z;
            let distance = (dx * dx + dy3 * dy3 + dz * dz).sqrt();
            let wait = (0.1 * distance) as u32;
            if distance > f32::EPSILON && wait > 0 {
                let scale = 10.0 / distance;
                let actor = entity
                    .actor_data_mut()
                    .expect("ladder-flight victim lost actor data");
                // Ladder falls accumulate in 3D world space (the flight
                // tick adds these to the cached 3D position and
                // re-derives the map position), so `increment_y` holds
                // the 3D world-y advance here — not the map-space
                // advance the generic flights store.  Deriving the map
                // advance up front would round differently from the
                // original's per-tick 3D accumulation.
                actor.active_flight = Some(crate::element::ActiveFlight {
                    geometry: crate::element::FlightGeometry::World3d,
                    increment_x: dx * scale,
                    increment_y: dy3 * scale,
                    increment_z: dz * scale,
                    goal_x: entry.point.x,
                    goal_y: entry.point.y,
                    goal_z: entry.z,
                    frames_remaining: wait.min(u16::MAX as u32) as u16,
                    // Ladder/wall fall: domino effect is not invoked.
                    antagonist: None,
                    goal_layer: entry.layer,
                    goal_sector: crate::position_interface::SectorHandle::new(entry.sector),
                    obstacle: entry
                        .obstacle
                        .and_then(crate::position_interface::ObstacleHandle::new),
                    ladder_fall: true,
                });
            }
        }

        tracing::debug!(
            entity = ?victim_id,
            ?low_entry,
            "Ladder/wall fall translated"
        );
    }

    /// Dispatch `Command::Fall` to an actor.
    ///
    /// Runs on the actor that the sub-sequence targets — i.e.
    /// whichever side of a broken carry is NOT the one currently
    /// handling a damage element.  Posture dictates which fall
    /// animation plays:
    ///
    /// - `OnShoulders`  → `FallingShoulders` (NonInterruptable,
    ///   carrier pointer cleared).
    /// - `CarryingOnShoulders` / `HelpingToClimb` →
    ///   `FallingBackUpright` (carrier wobble, carried pointer
    ///   cleared).
    /// - Anything else → no-op with a warning.
    ///
    /// The sequence element is marked terminated as soon as combat_anim
    /// is in place — the animation plays out via the per-frame combat
    /// animation tick and the sprite-completion callback transitions
    /// posture to `Lying` / `DeadBack` at the end. This matches the
    /// semantics of every other combat-animation dispatch in the port
    /// (priority lives on the actor, not the element).
    pub(crate) fn dispatch_fall(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let posture = self
            .get_entity(owner)
            .map(|e| e.element_data().posture)
            .unwrap_or_default();

        // Pick the fall animation by current posture and insert it
        // as an order on the element.  The order is consumed by
        // `do_next_order`; per-anim posture transitions are applied
        // by `apply_falling_completion_side_effect`.
        let fall_anim = match posture {
            Posture::OnShoulders => Some(OrderType::FallingShoulders),
            Posture::CarryingOnShoulders | Posture::HelpingToClimb => {
                Some(OrderType::FallingBackUpright)
            }
            _ => {
                // The reference asserts here.  We log a warning and
                // still queue a stumble.
                tracing::warn!(
                    entity = ?owner,
                    ?posture,
                    "dispatch_fall: called on non-shoulder posture"
                );
                Some(OrderType::FallingBackUpright)
            }
        };

        // Clearing the carrier link sets the direction goal to the
        // carrier's direction first.  Capture the carrier's direction
        // before dropping the back-reference so we can rewrite the
        // falling actor's goal afterward.
        let carrier_dir = if matches!(posture, Posture::OnShoulders) {
            self.get_entity(owner)
                .and_then(|e| e.human_data())
                .and_then(|h| h.carrier)
                .and_then(|cid| self.get_entity(cid))
                .map(|c| c.element_data().direction())
        } else {
            None
        };

        // Side-state cleanup: clear carrier/carried pointers and
        // unfreeze the carried actor before the order insertion.
        if let Some(entity) = self.world.entities.get_mut(owner) {
            match posture {
                Posture::OnShoulders => {
                    if let Some(human) = entity.human_data_mut() {
                        human.carrier = None;
                    }
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.execution_frozen = false;
                    }
                    // Mirror the carrier-clear direction-goal rewrite.
                    if let Some(d) = carrier_dir {
                        entity.element_data_mut().set_direction_goal(d);
                    }
                }
                Posture::CarryingOnShoulders | Posture::HelpingToClimb => {
                    if let Some(pc) = entity.pc_data_mut() {
                        pc.carried = None;
                    }
                }
                _ => {
                    if let Some(pc) = entity.pc_data_mut() {
                        pc.carried = None;
                    }
                }
            }
        }

        if let Some(anim) = fall_anim {
            // Mark NonInterruptable for OnShoulders only — carrier
            // wobble runs at normal priority.
            if matches!(posture, Posture::OnShoulders) {
                self.orders.sequence_manager.set_element_priority(
                    seq_id,
                    elem_idx,
                    crate::sequence::SequencePriority::NonInterruptable,
                );
            }
            // This Fall command is the partner half launched by
            // TranslateShoulderDamage. Original PC::Translate authors both
            // FallingShoulders and FallingBackUpright with
            // bComputeDirection=false.
            self.push_translated_damage_order((seq_id, elem_idx), anim);
            self.orders
                .sequence_manager
                .element_in_progress(seq_id, elem_idx);
        } else {
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
        }
    }

    /// Translate damage that lands on a PC mid-carry.
    ///
    /// Routes every damage type that strikes a `CarryingOnShoulders`,
    /// `OnShoulders`, or `HelpingToClimb` PC through the
    /// shoulder-fall path instead of the normal sword / hit / push
    /// handler.
    ///
    /// The victim side of the carry drives its own fall via
    /// `combat_anim` (the current damage element is already being
    /// dispatched, so we don't launch a second element for it).  The
    /// *partner* side gets a fresh `Command::Fall` sequence element
    /// launched through `SequenceManager::launch_element`.  That
    /// element is dispatched by `dispatch_fall` next tick and
    /// terminates once its combat_anim is in place.
    ///
    /// Posture-to-side mapping:
    /// - **OnShoulders** (victim is carried): victim plays
    ///   `FallingShoulders` (NonInterruptable), Fall command is
    ///   launched at the carrier so the carrier stumbles.
    /// - **CarryingOnShoulders / HelpingToClimb** (victim is the
    ///   carrier): victim plays a `FallingBackUpright` stumble;
    ///   Fall command is launched at the carried so the carried
    ///   falls.
    ///
    /// ## Known gap
    ///
    /// - The victim's fall animation runs via `combat_anim` rather
    ///   than as an order on the current damage element, because the
    ///   damage elements don't support multi-order chains.  The
    ///   sprite-completion callback in `animation.rs` flips posture
    ///   to `Lying` / `DeadBack` when the sprite terminates.
    pub(super) fn translate_shoulder_damage(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
    ) {
        // Shoulder translation calls virtual SayOuch. NPCs override it;
        // PCs inherit RHElementActorHuman's no-op and receive any hurt/death
        // speech from their SetLifePoints override at damage-apply time.
        if matches!(
            self.get_entity(victim_id),
            Some(Entity::Soldier(_) | Entity::Civilian(_))
        ) {
            self.say_ouch(sim, assets, victim_id, None);
        }

        // Read posture + carrier/carried relationships.
        let (posture, carrier_id, carried_id) = {
            let v = match self.get_entity(victim_id) {
                Some(e) => e,
                None => return,
            };
            let posture = v.element_data().posture;
            let carrier = v.human_data().and_then(|h| h.carrier);
            let carried = v.pc_data().and_then(|p| p.carried);
            (posture, carrier, carried)
        };

        // Partner receives a Fall sub-sequence — determined by which
        // side of the carry the victim is on. Own-side animation is
        // set directly further down because the damage element is
        // already dispatching this function.
        let partner_for_fall: Option<EntityId> = match posture {
            Posture::OnShoulders => carrier_id,
            Posture::CarryingOnShoulders | Posture::HelpingToClimb => carried_id,
            _ => None,
        };

        // Pick the victim's fall animation by posture: OnShoulders
        // → FallingShoulders (NonInterruptable); the carrier side
        // (CarryingOnShoulders / HelpingToClimb) → FallingBackUpright
        // stumble at normal priority.
        let (anim, priority) = match posture {
            Posture::OnShoulders => (
                Some(OrderType::FallingShoulders),
                crate::sequence::SequencePriority::NonInterruptable,
            ),
            Posture::CarryingOnShoulders | Posture::HelpingToClimb => (
                Some(OrderType::FallingBackUpright),
                crate::sequence::SequencePriority::NotYetSet,
            ),
            _ => {
                tracing::warn!(
                    entity = ?victim_id,
                    ?posture,
                    "translate_shoulder_damage called on non-shoulder posture"
                );
                (None, crate::sequence::SequencePriority::NotYetSet)
            }
        };

        // Clearing the carrier link rewrites the direction goal to
        // the carrier's direction first.  Capture the carrier's
        // direction now.
        let carrier_dir = if matches!(posture, Posture::OnShoulders) {
            carrier_id
                .and_then(|cid| self.get_entity(cid))
                .map(|c| c.element_data().direction())
        } else {
            None
        };

        // Side-state cleanup before queuing the animation.
        if let Some(entity) = self.world.entities.get_mut(victim_id) {
            match posture {
                Posture::OnShoulders => {
                    if let Some(human) = entity.human_data_mut() {
                        human.carrier = None;
                    }
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.execution_frozen = false;
                    }
                    if let Some(d) = carrier_dir {
                        entity.element_data_mut().set_direction_goal(d);
                    }
                }
                Posture::CarryingOnShoulders | Posture::HelpingToClimb => {
                    if let Some(pc) = entity.pc_data_mut() {
                        pc.carried = None;
                    }
                    // Translation leaves the shoulder posture intact. The
                    // FallingBackUpright execute handler applies the landing
                    // posture only when this actor reaches its next slot.
                }
                _ => {}
            }
        }

        if let Some(fall_anim) = anim {
            // Set priority to NonInterruptable BEFORE inserting the
            // FallingShoulders order — at translate time, not execute
            // time.  The FallingShoulders execute path *asserts* the
            // priority is already NonInterruptable.  We set it
            // eagerly here for the OnShoulders branch; the
            // carrier-wobble branch inherits the parent damage
            // element's priority.
            if matches!(
                priority,
                crate::sequence::SequencePriority::NonInterruptable
            ) {
                let (dseq, didx) = damage_element;
                self.orders
                    .sequence_manager
                    .set_element_priority(dseq, didx, priority);
            }
            self.queue_damage_anim(victim_id, damage_element, fall_anim);
        }

        // Launch the Fall sub-sequence for the partner.
        if let Some(partner_id) = partner_for_fall {
            let elem = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::Fall,
                Some(partner_id),
            );
            let partner_seq_id = self.launch_element(elem);
            // For the CarryingOnShoulders / HelpingToClimb branches
            // the partner (the *carried* body) also receives a roll
            // translation on its new Fall sequence element, so a body
            // dropped onto a slope rolls away from the collapse.  The
            // OnShoulders branch launches Fall on the *carrier* and
            // does not roll, so we gate on posture.
            if matches!(
                posture,
                Posture::CarryingOnShoulders | Posture::HelpingToClimb
            ) {
                self.try_queue_roll(assets, partner_id, (partner_seq_id, 0));
            }
        }

        // After appending the fall order, attempt a roll on the
        // victim's own damage element so a shoulder-damaged actor
        // landing on a slope rolls instead of stopping at the final
        // fall frame.  `try_queue_roll` is a no-op on flat terrain.
        self.try_queue_roll(assets, victim_id, damage_element);

        tracing::debug!(
            entity = ?victim_id,
            ?posture,
            partner = ?partner_for_fall,
            "Shoulder damage translated"
        );
    }

    /// Computes a strike-type-specific flight vector, validates the
    /// destination against walkable terrain, and sets up an
    /// `ActiveFlight` that advances the victim position each frame
    /// over the animation duration.
    pub(super) fn apply_push_effect(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        attacker_id: EntityId,
        push: &PushStrikeInfo,
        damage_result: combat::SwordDamageResult,
        damage_element: (crate::sequence::SequenceId, usize),
        death_cascade_already_applied: bool,
    ) -> bool {
        // NonInterruptable guard: if the victim is already playing a
        // non-interruptable sequence (an earlier falling-pushed /
        // rolling / ladder-wall fall), refuse to start a fresh push
        // on top of it.  We check the priority of the victim's
        // current InProgress sequence element.
        let current_priority = self
            .current_sequence_element_for_actor(victim_id)
            .and_then(|(s, i)| self.orders.sequence_manager.get_element(s, i))
            .map(|e| e.priority)
            .unwrap_or_default();
        if current_priority.is_non_interruptable()
            // Skip the guard for the damage element we're currently
            // dispatching — its NonInterruptable priority is on the
            // current element being processed, not a *prior* one.
            && Some(damage_element)
                != self.current_sequence_element_for_actor(victim_id)
        {
            tracing::debug!(
                victim = ?victim_id,
                attacker = ?attacker_id,
                "apply_push_effect: victim sequence is non-interruptable, skipping push visual"
            );
            // Still counted as "handled" so the caller's damage-path
            // branching stays consistent.
            return true;
        }

        // TranslatePushDamage calls the virtual SayOuch. NPCs override it,
        // while PCs inherit RHElementActorHuman's inline no-op.
        if matches!(
            self.get_entity(victim_id),
            Some(Entity::Soldier(_) | Entity::Civilian(_))
        ) {
            self.say_ouch(sim, assets, victim_id, None);
        }

        // Shoulder-posture victims route through
        // `translate_shoulder_damage` before falling through to the
        // base-class push-damage path.
        let victim_posture = self
            .get_entity(victim_id)
            .map(|e| e.element_data().posture)
            .unwrap_or_default();
        if matches!(
            victim_posture,
            Posture::OnShoulders | Posture::CarryingOnShoulders | Posture::HelpingToClimb
        ) {
            self.translate_shoulder_damage(sim, assets, victim_id, damage_element);
            return true;
        }

        // CarryingCorpse arm — drop the corpse instantly and fall
        // through to the base-class push-damage path which runs the
        // push flight machinery below.  Re-read the posture
        // afterwards so the downstream ladder/wall + flight
        // selection sees the carrier's new Upright posture.
        let victim_posture = if victim_posture == Posture::CarryingCorpse {
            self.force_drop_carried_corpse_instant(victim_id);
            self.get_entity(victim_id)
                .map(|e| e.element_data().posture)
                .unwrap_or_default()
        } else {
            victim_posture
        };

        // Entities on a ladder/wall get the ladder-fall variant
        // instead of the normal push flight.
        if matches!(victim_posture, Posture::OnLadder | Posture::OnWall) {
            self.translate_ladder_wall_fall(assets, victim_id, damage_element);
            return true;
        }

        let victim_is_rider = self
            .get_entity(victim_id)
            .is_some_and(|entity| matches!(entity, Entity::Soldier(s) if s.soldier.rider));

        // Read victim state for animation selection
        let (posture, action_state, is_dead, is_unconscious, concussion) = {
            let victim = match self.get_entity(victim_id) {
                Some(e) => e,
                None => return false,
            };
            let posture = victim.element_data().posture;
            let action = victim
                .actor_data()
                .map(|a| a.action_state)
                .unwrap_or_default();
            let dead = victim.is_dead();
            let unconscious = victim.human_data().map(|h| h.unconscious).unwrap_or(false);
            let conc = victim
                .human_data()
                .map(|h| h.concussion_of_the_brain)
                .unwrap_or(0);
            (posture, action, dead, unconscious, conc)
        };

        let push_anims =
            translated_push_damage_animations(posture, action_state, victim_is_rider, is_dead);

        if let Some(anims) = push_anims {
            // The falling sequence is marked non-interruptable so
            // incoming damage can't replace it mid-flight; the
            // falling, optional standup, and optional stunned
            // animations are inserted as chained orders on the
            // damage element so `do_next_order` plays them in
            // sequence.
            self.queue_damage_anim(victim_id, damage_element, anims.falling);
            self.orders
                .sequence_manager
                .get_element_mut(damage_element.0, damage_element.1)
                .and_then(|element| element.orders.back_mut())
                .expect("queued falling-pushed order vanished")
                .antagonist = Some(attacker_id);
            if !is_dead
                && !is_unconscious
                && damage_result.contains(combat::SwordDamageResult::STUNNING_DAMAGE)
            {
                if let Some(standup) = anims.standing_up {
                    self.push_translated_damage_order(damage_element, standup);
                }
                if let Some(stunned) = anims.stunned
                    && concussion > STUNNING_THRESHOLD
                {
                    self.push_translated_damage_order(damage_element, stunned);
                }
            }

            // Handle death/KO side effects without changing posture here.
            // `ExecuteFallingPushed` owns Flying on motion Start and the
            // DeadBack/Lying landing transition. `DyingUpright` likewise
            // owns the dead rider's posture transition on animation Start.
            if is_dead && !death_cascade_already_applied {
                let is_pc = if let Some(entity) = self.world.entities.get_mut(victim_id) {
                    if let Some(actor) = entity.actor_data_mut() {
                        clear_fatal_push_path(actor);
                    }
                    if let Some(npc) = entity.npc_data_mut() {
                        crate::ai_vision::set_view_status(npc, EyeStatus::DieOrGetUnconscious);
                        npc.alerted = false;
                    }
                    entity.kind().is_pc()
                } else {
                    false
                };
                self.quit_swordfight(sim, assets, victim_id);
                if is_pc {
                    // Run the PC kill cascade (gang removal, trumpet,
                    // new-PC stat decrement, macro burn, dead_pc
                    // gate) for the push-fatal path so it matches the
                    // damage-element death path.
                    self.apply_pc_kill_cascade(sim, assets, victim_id);
                }
            } else if is_unconscious {
                // Animated TranslatePushDamage always performs its plain
                // QuitSwordFight, including when the victim was already
                // unconscious before this hit. Only SetConcussion's fuller
                // KO cascade is conditional on the conscious-to-unconscious
                // transition.
                self.quit_swordfight(sim, assets, victim_id);
            }

            self.try_queue_roll(assets, victim_id, damage_element);

            tracing::debug!(
                victim = ?victim_id,
                attacker = ?attacker_id,
                repulsion = push.repulsion,
                ?posture,
                falling_anim = ?anims.falling,
                "Push effect applied (posture-aware)"
            );
            true // push handled everything
        } else {
            // No falling animation (already lying/dead/carried).
            if is_dead && !death_cascade_already_applied {
                let is_pc = if let Some(entity) = self.world.entities.get_mut(victim_id) {
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.clear_path();
                    }
                    if let Some(npc) = entity.npc_data_mut() {
                        crate::ai_vision::set_view_status(npc, EyeStatus::DieOrGetUnconscious);
                        npc.alerted = false;
                    }
                    entity.kind().is_pc()
                } else {
                    false
                };
                self.quit_swordfight(sim, assets, victim_id);
                if is_pc {
                    self.apply_pc_kill_cascade(sim, assets, victim_id);
                }
            }
            if let Some(posture) = translated_push_posture(false, is_dead, is_unconscious)
                && let Some(entity) = self.world.entities.get_mut(victim_id)
            {
                entity.set_posture(posture);
            }
            tracing::debug!(
                victim = ?victim_id,
                attacker = ?attacker_id,
                repulsion = push.repulsion,
                "Push effect applied (no animation — already down)"
            );
            true // still handled by push path
        }
    }

    /// Initialize `ReadyForTakeOff` for an authored falling-pushed order.
    /// Translation deliberately leaves no `active_flight`: the original
    /// samples the attacker's live direction and both actors' live positions
    /// under `ExecuteFallingPushed::IsInitialisation()`.
    pub(crate) fn initialize_push_flight(
        &mut self,
        assets: &LevelAssets,
        victim_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
        falling_anim: OrderType,
    ) {
        let (attacker_id, strike, profile_idx) = self
            .orders
            .sequence_manager
            .get_element(damage_element.0, damage_element.1)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Damage {
                    origin: Some(origin),
                    sword_strike: Some(strike),
                    sword_profile_idx: Some(profile_idx),
                    ..
                } => Some((*origin, *strike, *profile_idx)),
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!("falling-pushed actor {victim_id:?} lacks sword-damage inputs")
            });
        let profile = assets
            .profile_manager
            .get_hth_weapon(profile_idx)
            .unwrap_or_else(|| panic!("missing push weapon profile {profile_idx}"));
        let thrust = &profile.thrusts[strike as usize];

        let (attacker_pos, attacker_dir) = self
            .get_entity(attacker_id)
            .map(|entity| {
                (
                    entity.element_data().position_map(),
                    entity.element_data().direction(),
                )
            })
            .unwrap_or_else(|| panic!("falling-pushed attacker {attacker_id:?} is missing"));
        let (victim_pos, victim_z, layer, sector, move_box, rider, frames) = {
            let victim = self
                .get_entity(victim_id)
                .unwrap_or_else(|| panic!("falling-pushed victim {victim_id:?} is missing"));
            let concrete_anim = crate::engine::animation::sprite_anim_for_order(
                victim.sprite(),
                falling_anim,
                victim.is_pc(),
            );
            (
                victim.element_data().position_map(),
                victim.position_iface().get_elevation(),
                victim.element_data().layer(),
                victim.element_data().sector(),
                // Retail GetMoveBox(RHPOSTURE_LYING) currently returns the
                // primary move box (the posture switch is commented out).
                *victim.position_iface().get_move_box(),
                matches!(victim, Entity::Soldier(soldier) if soldier.soldier.rider),
                victim
                    .sprite()
                    .ready_for_takeoff_ticks_for_anim(concrete_anim)
                    .max(1),
            )
        };

        let direction = sector_to_vector_iso(attacker_dir as u16, ASPECT_RATIO);
        let (mut flight_x, mut flight_y) = if strike == SwordStrike::Charge {
            (
                direction.0 * thrust.repulsion as f32,
                direction.1 * thrust.repulsion as f32,
            )
        } else {
            push_flight_vector(
                thrust.kind,
                direction,
                attacker_pos,
                victim_pos,
                thrust.maximal_distance as f32,
            )
        };
        if flight_x.abs() > 0.01 || flight_y.abs() > 0.01 {
            let facing =
                (crate::position_interface::vector_to_sector_0_to_15(flight_x, flight_y) + 8) % 16;
            self.get_entity_mut(victim_id)
                .expect("falling-pushed victim vanished")
                .element_data_mut()
                .set_direction_instantly(facing);
        }
        if rider {
            flight_x = 0.0;
            flight_y = 0.0;
        }

        let candidate = |fraction: f32| {
            crate::coordinates::MapPoint::new(
                victim_pos.x + flight_x * fraction,
                victim_pos.y + flight_y * fraction,
            )
        };
        let authorized = |point| {
            self.world
                .fast_grid
                .is_straight_movement_authorized(victim_pos, point, layer, &move_box)
        };
        let chosen = if authorized(candidate(1.0)) {
            Some(candidate(1.0))
        } else if authorized(candidate(0.5)) {
            Some(if authorized(candidate(0.75)) {
                candidate(0.75)
            } else {
                candidate(0.5)
            })
        } else if authorized(candidate(0.25)) {
            Some(candidate(0.25))
        } else {
            None
        };
        let goal = chosen
            .filter(|point| {
                sector
                    .and_then(|sector| {
                        self.world
                            .fast_grid
                            .level
                            .sector_number_map
                            .get(&crate::sector::SectorNumber::new(sector.get() as i16))
                            .copied()
                    })
                    .and_then(|index| self.world.fast_grid.level.sectors.get(index))
                    .is_none_or(|sector| sector.contains_point(*point))
            })
            .unwrap_or(victim_pos);
        let (obstacle, goal_z) = sector
            .and_then(|sector| self.get_projection_area_index(assets, sector.get(), layer, goal))
            .map(|index| {
                let obstacle = crate::position_interface::ObstacleHandle::new(index);
                let z = self
                    .sight_obstacles(assets)
                    .get(index as usize)
                    .unwrap_or_else(|| panic!("push goal obstacle {index} is missing"))
                    .compute_top_z_from_projection(goal.x, goal.y);
                (obstacle, z)
            })
            .map_or((None, 0.0), |(obstacle, z)| (obstacle, z));

        // RHSprite::ReadyForTakeOff calls RHPositionInterface::SetObstacle,
        // not SetObstacleAndMaterial: install the goal plane immediately but
        // preserve the actor's current footstep material during the flight.
        let plane = obstacle.map(|handle| {
            let source = self
                .sight_obstacles(assets)
                .get(handle.get() as usize)
                .unwrap_or_else(|| panic!("push goal obstacle {} is missing", handle.get()));
            crate::position_interface::PlaneZCoeffs::from_plane_points(&source.top_plane_points)
        });
        let position = self
            .get_entity_mut(victim_id)
            .expect("falling-pushed victim vanished")
            .position_iface_mut();
        // Original ReadyForTakeOff captures `ptMyPosition3D` before it
        // installs the landing obstacle. SetObstacle only invalidates the
        // lazy 3D cache there; the following SetIncrement/UpdatePosition
        // continues from that captured takeoff point. Rust's eager
        // set_obstacle recomputation must therefore not replace the flight's
        // starting elevation with the landing plane's height.
        let takeoff_position = position.get_position();
        position.set_obstacle(obstacle, plane);
        position.set_position(takeoff_position);
        position.set_layer_goal(
            crate::position_interface::Layer::new(layer)
                .expect("falling-pushed victim layer cannot be the no-layer sentinel"),
        );
        let dx = goal.x - victim_pos.x;
        let dy = goal.y - victim_pos.y;
        let dz = goal_z - victim_z;
        let dy_world = (goal.y + goal_z) - (victim_pos.y + victim_z);
        let actor = self
            .get_entity_mut(victim_id)
            .expect("falling-pushed victim vanished")
            .actor_data_mut()
            .expect("falling-pushed victim lost actor data");
        actor.active_flight = (dx.abs() > 0.01 || dy.abs() > 0.01 || dz.abs() > 0.01).then_some(
            crate::element::ActiveFlight {
                geometry: crate::element::FlightGeometry::World3d,
                increment_x: dx / frames as f32,
                increment_y: dy_world / frames as f32,
                goal_x: goal.x,
                goal_y: goal.y,
                frames_remaining: frames,
                antagonist: Some(attacker_id),
                increment_z: dz / frames as f32,
                goal_z,
                goal_layer: layer,
                goal_sector: sector,
                obstacle,
                ladder_fall: false,
            },
        );
    }

    // ─── Rolling on slopes ──────────────────────────────────────────

    /// Compute the top-plane normal for a sight obstacle from its three
    /// defining points.  Returns `[nx, ny, nz]`.
    ///
    pub(super) fn obstacle_top_normal(obstacle: &crate::sight_obstacle::SightObstacle) -> [f32; 3] {
        let [p0, p1, p2] = obstacle.top_plane_points;
        // Two edge vectors
        let u = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let v = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        // Cross product
        let nx = u[1] * v[2] - u[2] * v[1];
        let ny = u[2] * v[0] - u[0] * v[2];
        let nz = u[0] * v[1] - u[1] * v[0];
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        if len < 1e-6 {
            return [0.0, 0.0, 1.0]; // flat
        }
        let inv = 1.0 / len;
        // Ensure normal points upward (positive Z)
        if nz * inv >= 0.0 {
            [nx * inv, ny * inv, nz * inv]
        } else {
            [-nx * inv, -ny * inv, -nz * inv]
        }
    }

    /// Check if the entity is on a slope steep enough to roll.
    ///
    /// Returns the obstacle's top-plane normal if rolling is needed
    /// (normal.z <= 0.76, i.e. ~cos(40°)).
    pub(super) fn get_roll_normal(
        &self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) -> Option<[f32; 3]> {
        let obstacle_idx = self
            .get_entity(entity_id)?
            .element_data()
            .obstacle_index()?;

        let obstacle = self
            .sight_obstacles(assets)
            .get(usize::from(obstacle_idx))?;

        let normal = Self::obstacle_top_normal(obstacle);
        // Roll is needed when normal.z <= 0.76 (~cos(40°)).
        if normal[2] <= 0.76 {
            Some(normal)
        } else {
            None
        }
    }

    /// Compute a roll destination point from the obstacle normal.
    ///
    /// The roll direction is the downhill direction derived from the surface
    /// normal projected onto the ground plane, scaled by 100 map units.
    /// When `check_increment` is true, the function also refuses to return a
    /// destination whose roll direction opposes the entity's current
    /// movement increment (used by `update_roll` to stop rolling against the
    /// slope when the entity has been redirected).
    ///
    pub(super) fn find_roll_point(
        &self,
        entity_id: EntityId,
        normal: [f32; 3],
        check_increment: bool,
    ) -> Option<crate::coordinates::MapPoint> {
        let entity = self.get_entity(entity_id)?;
        // Only humans can roll; gate on actor presence (drops the
        // pre-existing actor check while keeping the same coverage).
        entity.actor_data()?;
        let pos = entity.element_data().position_map();
        let layer = entity.element_data().layer();

        // Compute roll direction from normal.
        let mut rx = normal[0];
        let mut ry = normal[1];
        let mut rz = normal[2];

        if rz > 0.0 {
            rz -= 1.0 / rz;
        } else {
            rz -= 100.0;
        }

        // Apply aspect ratio to Y.
        ry *= combat::ASPECT_RATIO;

        // Scale by 100.
        rx *= 100.0;
        ry *= 100.0;
        rz *= 100.0;

        let push_map = crate::coordinates::MapVec::from_world_xyz(rx, ry, rz);

        // If the entity is already moving against the roll direction
        // (dot product negative), refuse to redirect it.
        if check_increment && let Some(pi) = Some(entity.position_iface()) {
            let inc = pi.get_increment_map();
            if inc.x * push_map.x + inc.y * push_map.y < 0.0 {
                return None;
            }
        }

        let dest_x = pos.x + push_map.x;
        let dest_y = pos.y + push_map.y;

        // Translate the actor's live primary move box to the roll destination.
        // Original `GetMoveBox(RHPOSTURE_LYING)` currently returns `mboxMove`
        // unconditionally; its alternate-posture implementation is commented
        // out. `find_authorized_position_straight` mutates this box by pushing
        // it off intersecting motion lines, and the adjusted center becomes
        // the roll endpoint.
        let mut dest_box = roll_destination_box(
            *entity.position_iface().get_move_box(),
            crate::coordinates::MapPoint::new(dest_x, dest_y),
        );
        let pt_start = pos;
        if !self
            .world
            .fast_grid
            .find_authorized_position_straight(&mut dest_box, pt_start, layer)
        {
            return None;
        }
        Some(dest_box.center())
    }

    /// Queue a rolling animation after a fall if the entity is on a steep slope.
    ///
    pub(super) fn try_queue_roll(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
    ) {
        let normal = match self.get_roll_normal(assets, entity_id) {
            Some(n) => n,
            None => return,
        };
        let dest = match self.find_roll_point(entity_id, normal, false) {
            Some(d) => d,
            None => return,
        };

        // Append a Rolling order with the authoritative map destination.
        // Its Execute arm drives ordinary PerformMotion directly.
        self.push_translated_roll_order(damage_element, dest);
    }

    /// Append the positional order authored by Original TranslateRoll. Unlike
    /// the direct damage-reaction animations, this order retains RHOrder's
    /// default `bComputeDirection=true`.
    pub(super) fn push_translated_roll_order(
        &mut self,
        damage_element: (crate::sequence::SequenceId, usize),
        dest: crate::coordinates::MapPoint,
    ) {
        let (dseq, didx) = damage_element;
        let roll_order = crate::order::Order::new(
            OrderType::Rolling,
            dest.x,
            dest.y,
            self.orders.allocate_order_id(),
        );
        self.orders
            .sequence_manager
            .push_order_on(dseq, didx, roll_order);
    }

    // ─── Multi-target strike execution ──────────────────────────────

    /// Execute a sword strike that may hit multiple targets.
    ///
    /// Dispatches to the appropriate hit-detection method based on strike kind:
    /// - Straight: single-target distance check
    /// - Lateral: angular arc sweep
    /// - Push: rectangular area
    /// - Circle/half-circle: wide angular sweep
    ///
    /// Returns the list of victims actually hit.
    pub(super) fn execute_multi_target_strike(
        &mut self,
        assets: &LevelAssets,
        attacker_id: EntityId,
        strike: SwordStrike,
        profile_idx: Option<u32>,
    ) -> Vec<EntityId> {
        let profile = profile_idx
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .cloned();
        let profile = match profile {
            Some(p) => p,
            None => return Vec::new(),
        };

        let attacker_pos = self
            .get_entity(attacker_id)
            .map(|e| {
                let p = e.element_data().position_map();
                (p.x, p.y)
            })
            .unwrap_or((0.0, 0.0));
        let attacker_dir = self
            .get_entity(attacker_id)
            .map(|e| e.element_data().direction())
            .unwrap_or(0);

        let thrust = &profile.thrusts[strike as usize];
        let min_dist = thrust.minimal_distance as f32;
        let max_dist = thrust.maximal_distance as f32;
        let kind = thrust.kind;

        let obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };

        let mut victims = match kind {
            WeaponThrustKind::Straight | WeaponThrustKind::Assault => {
                // Single-target: check principal opponent / original target only
                // For sequence-driven strikes, the target is already known
                Vec::new() // handled by the existing single-target path
            }
            WeaponThrustKind::Lateral => {
                let dir_angle = sector_to_angle(attacker_dir);
                let strike_dir = thrust.direction;
                let (begin_sector, end_sector) = match strike_dir {
                    crate::profiles::WeaponThrustDirection::RightToLeft => {
                        let initial = dir_angle + strike_profile_angle(thrust.initial_angle);
                        let final_a = dir_angle - strike_profile_angle(thrust.final_angle);
                        (angle_to_sector(final_a), angle_to_sector(initial))
                    }
                    _ => {
                        let initial = dir_angle - strike_profile_angle(thrust.initial_angle);
                        let final_a = dir_angle + strike_profile_angle(thrust.final_angle);
                        (angle_to_sector(initial), angle_to_sector(final_a))
                    }
                };
                let attacker_position = self
                    .get_entity(attacker_id)
                    .map(|entity| entity.element_data().position())
                    .unwrap_or_else(|| {
                        panic!("lateral strike attacker {attacker_id:?} is missing")
                    });
                collect_lateral_strike_victims(
                    &self.world.entities,
                    attacker_id,
                    attacker_position,
                    min_dist,
                    max_dist,
                    begin_sector,
                    end_sector,
                    &assets.profile_manager,
                    &self.world.fast_grid,
                    obstacles,
                )
            }
            WeaponThrustKind::PushAside => {
                // RHElement::GetDirectionVector builds the literal 16-sector
                // vector with ASPECT_RATIO before ExecutePushSwordStrike
                // applies the shipping INVERSE_SWORDFIGHT_ASPECT_RATIO.
                // Using the ordinary unit-circle helper here rotates the
                // narrow push rectangle in map space and can reject actors
                // that Original includes near a side boundary.
                let half_width = push_strike_half_width(thrust.repulsion);
                let attacker = self
                    .get_entity(attacker_id)
                    .unwrap_or_else(|| panic!("push-strike attacker {attacker_id:?} is missing"));
                let attacker_elevation = attacker.position_iface().get_elevation();
                let attacker_ground = attacker.ground_position();
                collect_push_victims(
                    &self.world.entities,
                    &PushStrikeParams {
                        attacker_id,
                        attacker_pos: (attacker_ground.x, attacker_ground.y),
                        attacker_elevation,
                        position_space: PushStrikePositionSpace::Ground,
                        attacker_direction: attacker_dir,
                        min_distance: min_dist,
                        max_distance: max_dist,
                        half_width,
                    },
                    &assets.profile_manager,
                    &self.world.fast_grid,
                    obstacles,
                )
            }
            WeaponThrustKind::TrueHalfCircle | WeaponThrustKind::FalseHalfCircle => {
                // Half circle: ±90° from facing direction
                let dir_angle = sector_to_angle(attacker_dir);
                let strike_dir = thrust.direction;
                let (begin_sector, end_sector) = match strike_dir {
                    crate::profiles::WeaponThrustDirection::RightToLeft => {
                        let initial = dir_angle + strike_profile_angle(thrust.initial_angle);
                        let final_a = initial - std::f32::consts::PI;
                        (angle_to_sector(final_a), angle_to_sector(initial))
                    }
                    _ => {
                        let initial = dir_angle - strike_profile_angle(thrust.initial_angle);
                        let final_a = initial + std::f32::consts::PI;
                        (angle_to_sector(initial), angle_to_sector(final_a))
                    }
                };
                collect_arc_victims(
                    &self.world.entities,
                    attacker_id,
                    attacker_pos,
                    min_dist,
                    max_dist,
                    begin_sector,
                    end_sector,
                    &assets.profile_manager,
                    &self.world.fast_grid,
                    obstacles,
                )
            }
            WeaponThrustKind::TrueCircle | WeaponThrustKind::FalseCircle => collect_arc_victims(
                &self.world.entities,
                attacker_id,
                attacker_pos,
                min_dist,
                max_dist,
                0,
                15,
                &assets.profile_manager,
                &self.world.fast_grid,
                obstacles,
            ),
        };

        // Original seeds every multi-target strike by walking
        // `RHEngine::marrayActors` in `GetActor(i)` order
        // (`RHelementactorhuman.cpp`: lateral 9614+, with the same actor walk
        // in the push/circle executors). Entity-kind slots are not that FIFO:
        // loaded PCs can have an ID order different from their authored
        // insertion order. Preserve the actor walk before the retained sweep
        // (or immediate push effects) starts, since each victim's synchronous
        // damage callbacks can change how later victims are handled.
        victims.sort_by_key(|&victim_id| self.world.original_creation_order(victim_id));
        victims
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::{MapPoint, MoveBox};

    #[test]
    fn rolling_destination_uses_live_primary_move_box() {
        let live = MoveBox::from_coords(-6.0, -4.0, 6.0, 4.0);
        let translated = roll_destination_box(live, MapPoint::new(250.0, 1180.0));

        assert_eq!(translated.x_min(), 244.0);
        assert_eq!(translated.y_min(), 1176.0);
        assert_eq!(translated.x_max(), 256.0);
        assert_eq!(translated.y_max(), 1184.0);
        assert_eq!(translated.center(), MapPoint::new(250.0, 1180.0));
    }

    #[test]
    fn push_flight_vector_matches_all_original_supported_thrust_kinds() {
        let attacker = MapPoint::new(0.0, 0.0);
        let direction = (1.0, 0.0);

        assert_eq!(
            push_flight_vector(
                WeaponThrustKind::PushAside,
                direction,
                attacker,
                MapPoint::new(-3.0, 0.0),
                10.0,
            ),
            (13.0, 0.0),
            "PUSH_ASIDE uses the signed attacker-direction projection"
        );

        for kind in [WeaponThrustKind::TrueCircle, WeaponThrustKind::FalseCircle] {
            assert_eq!(
                push_flight_vector(kind, direction, attacker, MapPoint::new(0.0, 4.0), 10.0,),
                (0.0, 10.0),
                "{kind:?} moves radially but uses the attacker-direction projection"
            );
        }
    }

    #[test]
    fn circle_push_uses_original_geotype_norm_rounding() {
        let attacker = MapPoint::new(0.0, 0.0);
        let victim = MapPoint::new(46.609222, 53.838055);

        let vector = push_flight_vector(
            WeaponThrustKind::TrueCircle,
            (1.0, 0.0),
            attacker,
            victim,
            100.0,
        );

        assert_eq!(vector.0.to_bits(), 0x420b_c85a);
        assert_eq!(vector.1.to_bits(), 0x4221_764e);
        let hypot_distance = (victim.x - attacker.x).hypot(victim.y - attacker.y);
        let remaining = 100.0 - (victim.x - attacker.x);
        assert_eq!(
            ((victim.x - attacker.x) / hypot_distance * remaining).to_bits(),
            0x420b_c85b,
            "control must distinguish the platform hypot rounding"
        );
    }

    #[test]
    #[should_panic(expected = "unexpected push thrust kind: Straight")]
    fn push_flight_vector_rejects_unexpected_thrust_kind() {
        push_flight_vector(
            WeaponThrustKind::Straight,
            (1.0, 0.0),
            MapPoint::new(0.0, 0.0),
            MapPoint::new(4.0, 0.0),
            10.0,
        );
    }

    #[test]
    fn fatal_animated_push_defers_dead_posture_to_flight_landing() {
        let anims = translated_push_damage_animations(
            Posture::Upright,
            ActionState::WaitingSword,
            false,
            true,
        );

        assert_eq!(
            anims.map(|anims| anims.falling),
            Some(crate::order::OrderType::FallingPushedWithSword)
        );
        assert_eq!(translated_push_posture(anims.is_some(), true, false), None);
    }

    #[test]
    fn fatal_animated_push_preserves_action_until_fall_execute() {
        let mut actor = crate::element::ActorData {
            action_state: ActionState::WaitingSword,
            ..Default::default()
        };

        clear_fatal_push_path(&mut actor);

        assert_eq!(actor.action_state, ActionState::WaitingSword);
    }

    #[test]
    fn knockout_animated_push_defers_lying_posture_to_flight_landing() {
        let anims =
            translated_push_damage_animations(Posture::Upright, ActionState::Waiting, false, false);

        assert_eq!(
            anims.map(|anims| anims.falling),
            Some(crate::order::OrderType::FallingPushedUpright)
        );
        assert_eq!(translated_push_posture(anims.is_some(), false, true), None);
    }

    #[test]
    fn fatal_rider_keeps_dying_upright_override_without_translation_snap() {
        let anims = translated_push_damage_animations(
            Posture::Lying,
            ActionState::WaitingSword,
            true,
            true,
        );

        assert_eq!(
            anims.map(|anims| anims.falling),
            Some(crate::order::OrderType::DyingUpright),
            "the rider-dead override precedes the posture switch"
        );
        assert_eq!(translated_push_posture(anims.is_some(), true, false), None);
    }

    #[test]
    fn already_lying_push_applies_terminal_posture_during_translation() {
        let anims =
            translated_push_damage_animations(Posture::Lying, ActionState::Waiting, false, true);

        assert!(anims.is_none());
        assert_eq!(
            translated_push_posture(anims.is_some(), true, false),
            Some(Posture::Dead)
        );
        assert_eq!(
            translated_push_posture(anims.is_some(), false, true),
            Some(Posture::Lying)
        );
    }
}
