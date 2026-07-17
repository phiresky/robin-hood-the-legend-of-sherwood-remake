//! Enemy AI initialization and ticking.
//!
//! The per-tick `tick_enemy_ai` orchestrator is split across submodules
//! by phase grouping:
//!  - [`snapshots`] — phases P1..P2d: read-only views built once per tick.
//!  - [`detection`] — phases P2a, P3, P3b: per-NPC visibility passes.
//!  - [`post_detection`] — phases P4..P6d: alert dispatch, pursuit, drains.

mod detection;
mod post_detection;
mod snapshots;

use super::*;
use crate::ai::{AiContext, AiPerTickData, StimulusType};
use crate::ai_entity_view::{self, AiEntityViewMap, SharedAiEntityViews};
use crate::ai_vision;
use crate::coordinates::MapPoint;
use crate::element::{Camp, Detectable, DetectableType, Entity, EntityId, PcId, SoldierId};
use crate::engine::SimScratch;
use crate::entities::Entities;

/// Exact `ubFramePhase` computed by `RHElementActorNPC::Hourglass`.
/// `register_number` is the original creation/register ordering value.
pub(super) fn npc_hourglass_frame_phase(frame: u32, register_number: u32) -> u8 {
    (frame as u8).wrapping_sub((register_number as u8).wrapping_add(100))
}

/// Number of arrows given to Merry Man archers in forest levels.
const MERRY_MAN_ARROWS: u16 = 3;

/// Snapshot of a potential detectable human at level-load time.
///
/// Used by [`EngineInner::init_one_ai`] to filter which other humans each
/// NPC should start with in its `detectable_lists[Enemy]` array —
/// the "create list of detectable enemies" pass inside the per-NPC
/// init for both enemy and friendly AI.
#[derive(Debug, Clone, Copy)]
struct PotentialDetectable {
    id: EntityId,
    is_pc: bool,
    is_soldier: bool,
    camp: Camp,
}

/// Build a snapshot of every live human in the engine.  Called once at
/// the start of [`EngineInner::init_ai`] and handed to every per-NPC init
/// pass.
fn build_potential_detectables(engine: &EngineInner) -> Vec<PotentialDetectable> {
    let mut out = Vec::new();
    for (id, entity) in engine.entities.humans() {
        if !entity.element_data().active {
            continue;
        }
        match entity {
            Entity::Pc(_) => {
                out.push(PotentialDetectable {
                    id: id.into(),
                    is_pc: true,
                    is_soldier: false,
                    // All PCs are Royalists.
                    camp: Camp::Royalists,
                });
            }
            Entity::Soldier(s) => {
                out.push(PotentialDetectable {
                    id: id.into(),
                    is_pc: false,
                    is_soldier: true,
                    camp: s.soldier.cached_camp,
                });
            }
            Entity::Civilian(c) => {
                // Civilians are tracked in the snapshot so the `IsFriend`
                // filter below can consider them, but the non-civilian
                // guard and the per-self filters in `add_detectable`
                // (Good/Evil branches) end up excluding every civilian
                // from every NPC's enemy list anyway.
                out.push(PotentialDetectable {
                    id: id.into(),
                    is_pc: false,
                    is_soldier: false,
                    camp: c.civilian.cached_camp,
                });
            }
            _ => {}
        }
    }
    out
}

/// Build this NPC's initial `detectable_lists[Enemy]` from a
/// [`PotentialDetectable`] snapshot.
///
/// Applies the combined filter of the enemy/friendly per-NPC init
/// (the outer loop over all humans, skipping friends and civilians in
/// the enemy case; adding PCs and opposing soldiers in the friendly
/// case) and then the per-self-type filter in `add_detectable`.
/// The net result for each self class:
///
/// - Royalist soldier (Merry Man): detects Lacklandist soldiers.
/// - Lacklandist soldier: detects Royalist soldiers + PCs.
/// - Royalist civilian: detects PCs.
/// - Lacklandist civilian (hostile civ): detects PCs.
fn build_detectable_enemies_for(
    self_camp: Camp,
    self_is_civilian: bool,
    self_id: EntityId,
    snapshot: &[PotentialDetectable],
) -> Vec<Detectable> {
    let mut out = Vec::new();
    for pd in snapshot {
        if pd.id == self_id {
            continue;
        }
        // Civilians are never added as detectables on any list (both
        // malignity and bonhomie init paths skip them via the kind
        // check / AddDetectable class filter).
        let pd_is_civilian = !pd.is_pc && !pd.is_soldier;
        if pd_is_civilian {
            continue;
        }
        let is_detectable = if self_is_civilian {
            // Bonhomie (friendly civilian) AddDetectable case:
            // "Good civilists - detect PCs" / "Evil civilists - detect PCs".
            // Royalist soldiers passed through the bonhomie outer filter
            // for lacklandist civilians are then rejected by AddDetectable.
            pd.is_pc
        } else {
            // Malignity (enemy soldier) AddDetectable cases:
            // - Royalist (Good) soldier → detects enemy (Lacklandist) soldiers.
            // - Lacklandist (Evil) soldier → detects good (Royalist) soldiers
            //   AND PCs.
            match self_camp {
                Camp::Royalists => pd.is_soldier && pd.camp == Camp::Lacklandists,
                Camp::Lacklandists => pd.is_pc || (pd.is_soldier && pd.camp == Camp::Royalists),
                Camp::Error => false,
            }
        };
        if is_detectable {
            out.push(Detectable {
                element: Some(pd.id),
                detectable_type: DetectableType::Enemy,
                seen_last_frame: false,
                heard_last_frame: false,
                seen_now: false,
                shadow_seen_now: false,
                shadow_seen_last_frame: false,
                last_visibility: 0.0,
            });
        }
    }
    out
}

/// Per-segment obstacle check against a hiking path's waypoints.
///
/// Each adjacent pair of waypoints that stays on the same sector/level
/// is tested for both raw motion reachability and thick-mobile
/// straight-movement authorization using the NPC's move box.  Returns
/// `true` when every applicable segment passes both checks.
///
/// Uses the "set `path_is_ok = false`, continue the loop" idiom so every
/// bad segment is logged rather than only the first. The debug-overlay
/// side effect (bad path visualisation) is dev-only and not yet
/// ported — log emission is the equivalent.
fn test_hiking_path_fine(
    grid: &crate::fast_find_grid::FastFindGrid,
    waypoints: &[crate::level_data::RawWaypoint],
    move_box: &crate::coordinates::MoveBox,
) -> bool {
    if waypoints.len() < 2 {
        return true;
    }
    let mut ok = true;
    let mut prev = &waypoints[0];
    for (i, wp) in waypoints.iter().enumerate().skip(1) {
        if wp.level == prev.level && wp.sector == prev.sector {
            let p1 = MapPoint::new(prev.x as f32, prev.y as f32);
            let p2 = MapPoint::new(wp.x as f32, wp.y as f32);
            if !grid.is_reachable_thin(p1, p2, wp.level) {
                tracing::debug!(
                    wp_idx = i,
                    p1 = ?p1,
                    p2 = ?p2,
                    layer = wp.level,
                    "TestIfPathIsFine: segment not reachable (obstacle)"
                );
                ok = false;
            }
            // Split the authorized check into its two components
            // (destination-box auth check, then thick-corridor check) so
            // diagnostics pinpoint which half of the test rejects.
            let dest_box = move_box.translated(p2);
            if !grid.is_position_authorized(&dest_box, wp.level) {
                tracing::debug!(
                    wp_idx = i,
                    p1 = ?p1,
                    p2 = ?p2,
                    layer = wp.level,
                    ?dest_box,
                    "TestIfPathIsFine: destination move-box overlaps obstacle \
                     (IsPositionAutorized)"
                );
                ok = false;
            }
            let hd =
                crate::coordinates::MoveBoxHalfDiagonal::new(move_box.x_max(), move_box.y_max());
            if !grid.is_reachable_thick(p1, p2, wp.level, hd) {
                tracing::debug!(
                    wp_idx = i,
                    p1 = ?p1,
                    p2 = ?p2,
                    layer = wp.level,
                    ?hd,
                    "TestIfPathIsFine: thick-corridor too close to obstacle \
                     (IsReachableThick)"
                );
                ok = false;
            }
        }
        prev = wp;
    }
    ok
}

/// Extract a [`ForecastInput`] from an entity for destination prediction.
///
/// Returns `None` for entities without actor data (e.g. objects, FX).
pub(super) fn extract_forecast_input(entity: &Entity) -> Option<crate::ai::ForecastInput> {
    let elem = entity.element_data();
    let actor = entity.actor_data()?;
    let door_pass = actor
        .active_door_pass
        .as_ref()
        .map(|dp| (dp.door_index, dp.direct));
    let forecasted_z = entity.position_iface().get_forecasted_movement().z;
    Some(crate::ai::ForecastInput {
        position_map_x: elem.position_map().x,
        position_map_y: elem.position_map().y,
        sector: elem.sector().map(u16::from).unwrap_or(0),
        layer: elem.layer(),
        direction: elem.direction() as u16,
        forecasted_movement_z: forecasted_z,
        door_pass,
    })
}

/// Build an [`AiContext`] from a generic [`Entity`] reference.
///
/// Extracts position, direction, posture, camp, building status, and
/// swordfighting flag from the live human opponent list so the AI think method
/// sees a consistent, non-stale snapshot each call.
///
/// Also threads the per-tick [`SharedAiEntityViews`] map into the
/// context so handlers can resolve arbitrary entity handles to live
/// position / state without a mutable engine borrow.  Callers grab
/// the map from [`SimScratch`], built by
/// [`EngineInner::build_sim_scratch`] before each dispatch pass.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_ai_context_from_entity(
    entity: &Entity,
    frame: u32,
    building_sector: Option<crate::position_interface::SectorHandle>,
    is_forest_level: bool,
    standard_view_polygon_radius: u16,
    entity_views: &SharedAiEntityViews,
    sight_obstacles: &crate::sight_obstacle::SharedSightObstacles,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    hiking_paths: &std::sync::Arc<Vec<crate::level_data::RawHikingPath>>,
    all_soldier_handles: &std::sync::Arc<Vec<u32>>,
) -> AiContext {
    let elem = entity.element_data();
    let camp = match entity {
        Entity::Soldier(s) => s.soldier.cached_camp,
        Entity::Civilian(c) => c.civilian.cached_camp,
        _ => crate::element::Camp::default(),
    };
    let actor = entity.actor_data();
    // `is_swordfighting` is "opponents list is non-empty"; do not proxy
    // it through action_state.
    let is_swordfighting = entity
        .human_data()
        .map(|h| !h.opponents.is_empty())
        .unwrap_or(false);
    let move_box = if actor.is_some() {
        *entity.position_iface().get_move_box()
    } else {
        Default::default()
    };
    let remaining_arrows = match entity {
        Entity::Soldier(s) => s.npc.number_of_arrows,
        _ => 0,
    };
    // `self_is_beggar` / `self_is_child` are civilian-type checks.
    // Non-civilian NPCs always read false (callers cast to civilian
    // first).
    let (self_is_beggar, self_is_child) = match entity {
        Entity::Civilian(c) => (
            c.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar,
            c.civilian.cached_civilian_type == crate::profiles::CivilianType::Child,
        ),
        _ => (false, false),
    };
    // Soldier vs civilian — drives the soldier-only macro opcodes
    // (CMD_CHECK_4, CMD_LOOK_LEFT, CMD_BEND, CMD_PATROL_*) which error
    // on civilians.
    let self_is_soldier = matches!(entity, Entity::Soldier(_));
    // `self_is_rider` is the cached `SoldierData.rider` flag from the
    // soldier profile, set at level load.  Non-soldier NPCs are never
    // riders.
    let self_is_rider = matches!(entity, Entity::Soldier(s) if s.soldier.rider);
    // `self_rank` / `self_pride` are the soldier's profile rank and
    // pride, used by the bored-time picker for longer officer/pride
    // bored intervals.  `ProfileRank::None` for non-soldiers makes the
    // officer check fall through.
    let (self_rank, self_pride) = match entity {
        Entity::Soldier(s) => {
            let rank = s
                .npc
                .ai_brain
                .enemy()
                .map(|e| e.soldier_profile_rank)
                .unwrap_or(crate::profiles::ProfileRank::None);
            let pride = s
                .npc
                .ai_brain
                .enemy()
                .map(|e| e.soldier_profile_pride)
                .unwrap_or(0);
            (rank, pride)
        }
        _ => (crate::profiles::ProfileRank::None, 0),
    };
    // Number of detectables of type Friend — the
    // `return_to_duty_common_stuff` guard uses this to decide whether
    // to clear the stashed detected body.
    let self_detectable_friend_count = entity
        .npc_data()
        .and_then(|npc| {
            npc.detectable_lists
                .get(crate::element::DetectableType::Friend as usize)
        })
        .map(|lst| lst.len() as u16)
        .unwrap_or(0);
    // Number of detectables of type MissedFriend — enemy
    // `return_to_duty` uses this to know whether to record the
    // abandoned checkpoint Charly in the missed-in-action list.
    let self_detectable_missed_friend_count = entity
        .npc_data()
        .and_then(|npc| {
            npc.detectable_lists
                .get(crate::element::DetectableType::MissedFriend as usize)
        })
        .map(|lst| lst.len() as u16)
        .unwrap_or(0);
    // The live animation/order currently playing on the actor.  Stored
    // as `actor.old_action` (`pub type Animation = OrderType`).
    let self_animation = actor.map(|a| a.old_action).unwrap_or_default();
    // Only soldiers can be forced-attentive; civilians always read
    // `false`.  Threaded into AiContext so
    // `set_alert_status_with_flags` can apply the view-override from
    // inside shared `AiController` paths.
    let self_forced_attentive = entity
        .npc_data()
        .and_then(|npc| npc.ai_brain.enemy())
        .is_some_and(|enemy| enemy.forced_attentive);
    let self_view_radius = entity
        .npc_data()
        .map(|npc| npc.view_radius as f32)
        .unwrap_or(standard_view_polygon_radius as f32);
    AiContext {
        position: crate::ai::Position {
            x: elem.position_map().x,
            y: elem.position_map().y,
            sector: elem.sector(),
            level: elem.layer(),
        },
        frame,
        direction: elem.direction() as u16,
        posture: elem.posture,
        in_uninterruptible_command: false,
        // `is_inside_building`: the building sector check OR the
        // door-transit branch — true during the few frames an actor is
        // on a door whose inside-sector is a building but whose current
        // sector pointer has not yet been swapped.
        in_building: building_sector.is_some() || entity.is_in_door_transit(),
        building_sector,
        camp,
        is_swordfighting,
        enter_swordfight_pending: false,
        is_forest_level,
        move_box,
        remaining_arrows,
        sq_standard_view_radius: (standard_view_polygon_radius as f32)
            * (standard_view_polygon_radius as f32),
        sq_self_view_radius: self_view_radius * self_view_radius,
        elevation: if actor.is_some() {
            entity.position_iface().get_elevation()
        } else {
            elem.position().z
        },
        self_is_beggar,
        self_is_child,
        self_is_soldier,
        self_is_rider,
        self_action_state: actor.map(|a| a.action_state).unwrap_or_default(),
        self_rank,
        self_pride,
        self_is_dead: entity.is_dead(),
        self_detectable_friend_count,
        self_detectable_missed_friend_count,
        self_forced_attentive,
        self_animation,
        antagonist: None,
        entity_views: entity_views.clone(),
        sight_obstacles: sight_obstacles.clone(),
        fast_grid: fast_grid.clone(),
        hiking_paths: hiking_paths.clone(),
        all_soldier_handles: all_soldier_handles.clone(),
    }
}

/// Look up the live metadata for an enemy's `primary_target` from the
/// engine entity table. Returns `(position, posture, current
/// animation, optional carrier position when the target is on
/// another entity's shoulders)`. Used by the per-tick caller to
/// populate [`AiPerTickData::primary_target_position`] and its
/// siblings so [`EnemyAi::reconsider_enemy_approach`] sees the live
/// target's position, posture, and current order.
///
/// Returns `None` when `target_id` is zero (unassigned target) or the
/// target slot is vacant. The caller should leave the tick fields
/// `None`/`false` in that case — `reconsider_enemy_approach` falls
/// back to the stored `seek_position`.
type PrimaryTargetMetadata = (
    crate::ai::Position,
    crate::element::Posture,
    Option<crate::order::OrderType>,
    Option<crate::ai::Position>,
    Option<crate::ai::HumanHandle>,
);

pub(super) fn lookup_primary_target_metadata(
    entities: &crate::entities::Entities,
    sequence_manager: &crate::sequence::SequenceManager,
    target_id: crate::element::EntityId,
) -> Option<PrimaryTargetMetadata> {
    if target_id.index() == 0 {
        return None;
    }
    let target = entities.get(target_id)?;
    let elem = target.element_data();
    let position = crate::ai::Position {
        x: elem.position_map().x,
        y: elem.position_map().y,
        sector: elem.sector(),
        level: elem.layer(),
    };
    let posture = elem.posture;
    // Orders live on the target's owning `SequenceElement.orders` —
    // look up the current in-progress element for the target actor.
    let animation = sequence_manager
        .current_order_for_actor(target_id)
        .map(|(_, _, o)| o.order_type);
    // Target-on-shoulders: retarget to the carrier.  Expose both the
    // carrier's handle (so the AI can re-point `primary_target` for the
    // friend-swap / focus / begin-swordfight reads) and the carrier's
    // position (used to recompute `live_target_pos`).  The carrier
    // entity id is tracked on `actor.carrier` when posture ==
    // OnShoulders.
    let (carrier_position, carrier_handle) =
        if matches!(posture, crate::element::Posture::OnShoulders) {
            target
                .human_data()
                .and_then(|h| h.carrier)
                .and_then(|c| {
                    entities.get(c).map(|carrier| {
                        let c_elem = carrier.element_data();
                        let pos = crate::ai::Position {
                            x: c_elem.position_map().x,
                            y: c_elem.position_map().y,
                            sector: c_elem.sector(),
                            level: c_elem.layer(),
                        };
                        (Some(pos), Some(c.index()))
                    })
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
        };
    Some((
        position,
        posture,
        animation,
        carrier_position,
        carrier_handle,
    ))
}

/// Build the list of same-camp friend candidates for the target-swap
/// heuristic in `ReconsiderEnemyApproach`.
///
/// Only soldiers currently in one of the approach substates
/// (`ATTACKING_RUNNING_TO_ENEMY`, `ATTACKING_WALKING_TO_ENEMY`,
/// `ATTACKING_CHARGING_ENEMY`) with a live primary target are
/// eligible.
pub(super) fn build_friend_swap_candidates(
    entities: &Entities,
    me_id: impl Into<crate::element::EntityId>,
    my_camp: crate::element::Camp,
) -> Vec<crate::ai::FriendSwapCandidate> {
    let me_id = me_id.into();
    let mut out = Vec::new();
    for (friend_id, s) in entities.soldiers() {
        if friend_id == me_id {
            continue;
        }
        if s.soldier.cached_camp != my_camp {
            continue;
        }
        let substate = s.npc.ai_substate();
        if !matches!(
            substate,
            crate::ai::Substate::AttackingRunningToEnemy
                | crate::ai::Substate::AttackingWalkingToEnemy
                | crate::ai::Substate::AttackingChargingEnemy
        ) {
            continue;
        }
        let friend_target_handle = match s
            .npc
            .ai_brain
            .base()
            .map(|ai| ai.primary_target)
            .unwrap_or(0)
        {
            0 => continue,
            h => h,
        };
        let friend_target_id =
            crate::element::EntityId::Pc(crate::entity_id::PcId(friend_target_handle));
        let friend_target = entities.get(friend_target_id);
        let Some(friend_target_entity) = friend_target else {
            continue;
        };
        let friend_pos = crate::ai::Position {
            x: s.element.position_map().x,
            y: s.element.position_map().y,
            sector: s.element.sector(),
            level: s.element.layer(),
        };
        let ft_elem = friend_target_entity.element_data();
        let friend_target_pos = crate::ai::Position {
            x: ft_elem.position_map().x,
            y: ft_elem.position_map().y,
            sector: ft_elem.sector(),
            level: ft_elem.layer(),
        };
        out.push(crate::ai::FriendSwapCandidate {
            friend_id: friend_id.into(),
            friend_position: friend_pos,
            friend_primary_target: friend_target_handle,
            friend_primary_target_position: friend_target_pos,
        });
    }
    out
}

/// Run the "avenger on the roof" wait-position lookup for the
/// evaluating NPC, if its `couldnt_reachpoint` flag is set.
///
/// The pre-dispatch wiring for
/// `get_avenger_on_the_roof_wait_position`.  The gate-chain walker
/// itself lives in [`crate::gate::compute_avenger_wait_position`];
/// this helper extracts the per-actor state the walker needs from
/// the live entity store.
///
/// Returns `None` when any input is missing or the walker finds no
/// blocking gate — the caller should leave
/// `tick.avenger_on_roof_wait_position` as `None` in that case.
pub(super) fn precompute_avenger_on_roof_wait_position(
    entities: &crate::entities::Entities,
    doors: &[crate::gate::Door],
    me_id: impl Into<crate::element::EntityId>,
    target_id: impl Into<crate::element::EntityId>,
    sector_lift_type: &impl Fn(crate::sector::SectorNumber) -> Option<crate::sector::LiftType>,
) -> Option<crate::ai::Position> {
    let me_id = me_id.into();
    let target_id = target_id.into();
    if doors.is_empty() {
        return None;
    }
    let me = entities.get(me_id)?;
    let target = entities.get(target_id)?;

    let me_elem = me.element_data();
    let target_elem = target.element_data();
    let me_sector = me_elem.sector()?;
    let target_sector = target_elem.sector()?;
    if me_sector == target_sector {
        return None;
    }

    let me_auth = me.actor_auth_info();
    let target_auth = target.actor_auth_info();

    let wait = crate::gate::compute_avenger_wait_position(
        doors,
        (target_elem.position_map().x, target_elem.position_map().y),
        target_sector.into(),
        &target_auth,
        (me_elem.position_map().x, me_elem.position_map().y),
        me_sector.into(),
        &me_auth,
        sector_lift_type,
    )?;

    Some(crate::ai::Position {
        x: wait.x,
        y: wait.y,
        sector: crate::position_interface::SectorHandle::new(wait.sector),
        level: wait.layer,
    })
}

/// Build a `MyExitDoorInfo` snapshot from the AI's stashed
/// `my_door_index`.  Strict semantics: returns `None` when no door has
/// been stashed upstream.  The stash is set by paths that explicitly
/// choose an exit door (MerryMan flee, RunAndAlertSoldiers); a
/// directly-invoked indoor AlertSoldiers without an upstream stash
/// refuses to project gather slots.
pub(super) fn build_my_exit_door_info(
    stashed_index: Option<u32>,
    doors: &[crate::gate::Door],
) -> Option<crate::ai::MyExitDoorInfo> {
    use crate::ai::MyExitDoorInfo;
    let idx = stashed_index?;
    let door = doors.get(idx as usize)?;
    let sector_out = crate::position_interface::SectorHandle::new(u16::from(door.sector_out));
    let position_out = crate::ai::Position {
        x: door.point_out.x,
        y: door.point_out.y,
        sector: sector_out,
        level: door.layer_out,
    };
    Some(MyExitDoorInfo {
        point_out: door.point_out,
        point_mid: door.point_mid,
        layer_out: door.layer_out,
        sector_out,
        position_out,
    })
}

/// Build the per-tick [`SharedAiEntityViews`] map from the live
/// entity store.
///
/// Called by [`EngineInner::build_sim_scratch`] at the start of each
/// AI dispatch pass so the map reflects current entity
/// positions / states.  Includes every PC, soldier, civilian, and
/// pickup-style bonus entity; skips inactive and projectile entities
/// (they're never the target of a handle lookup in the ported AI
/// paths).
pub(super) fn build_entity_views(engine: &EngineInner) -> AiEntityViewMap {
    let doors_ref = engine
        .mission_script
        .as_ref()
        .and_then(|s| s.game_host())
        .map(|gh| gh.doors.as_slice())
        .unwrap_or(&[]);

    // Pre-scan nets for `compute_nets_covering_me` reverse index:
    // victim entity-id → list of covering nets.  Per-victim loop:
    // iterate every net entity, include those whose `victims` contains
    // the probed human.  Doing it once up-front amortises the scan
    // across every stuck-victim view.
    //
    // Net radius: 10 when crumpled, else 40.
    let mut nets_by_victim: std::collections::HashMap<u32, Vec<ai_entity_view::NetCoverInfo>> =
        std::collections::HashMap::new();
    for (net_id, net) in engine.entities.nets() {
        if !net.element.active {
            continue;
        }
        if net.net.victims.is_empty() {
            continue;
        }
        let net_pos = net.element.position_map();
        let info = ai_entity_view::NetCoverInfo {
            handle: net_id.index(),
            position: crate::ai::Position {
                x: net_pos.x,
                y: net_pos.y,
                sector: net.element.sector(),
                level: net.element.layer(),
            },
            radius: if net.net.crumpled { 10.0 } else { 40.0 },
        };
        for victim in &net.net.victims {
            nets_by_victim.entry(victim.index()).or_default().push(info);
        }
    }

    let mut map = AiEntityViewMap::with_capacity(engine.entities.len());
    for (entity_id, entity) in engine.entities.occupied() {
        let elem = entity.element_data();
        if !elem.active {
            continue;
        }
        match entity {
            Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_) | Entity::Bonus(_) => {}
            _ => continue,
        }
        // Resolve building sector (if any) through the same helper
        // used by the existing AiContext building logic.
        let building_sector = engine.entity_building_sector(elem.sector());
        let mut view = ai_entity_view::entity_view_from_entity(
            entity,
            building_sector.is_some(),
            building_sector,
            engine.campaign.as_ref(),
        );

        // Door-rail snap: while a human actor is passing a door, AI
        // probes read the rail-anchored destination (point_in /
        // point_out) instead of the animated interpolated map
        // position.  `direct = true` (outside → inside) maps to
        // `point_in`; `direct = false` maps to `point_out`.
        if let Some(actor) = entity.actor_data()
            && let Some(dp) = actor.active_door_pass.as_ref()
            && let Some(door) = doors_ref.get(dp.door_index.0 as usize)
        {
            let rail_point = if dp.direct {
                door.point_in
            } else {
                door.point_out
            };
            view.position.x = rail_point.x;
            view.position.y = rail_point.y;
        }

        // PC riding on someone's shoulders reports the carrier's
        // position, not its own stale map slot.  `HumanData::carrier`
        // stores the carrier entity id; look it up and copy its map
        // position.
        if let Entity::Pc(pc) = entity
            && pc.element.posture == crate::element::Posture::OnShoulders
            && let Some(carrier_id) = pc.human.carrier
            && let Some(carrier) = engine.entities.get(carrier_id)
        {
            let cp = carrier.element_data().position_map();
            view.position.x = cp.x;
            view.position.y = cp.y;
        }

        // Attach pre-scanned covering nets for stuck victims, consumed
        // by `RunToFreeNetVictim`.
        if view.stuck_under_net
            && let Some(nets) = nets_by_victim.remove(&entity_id.index())
        {
            view.covering_nets = nets;
        }

        // Pre-compute the destination the actor is heading toward so
        // AI handlers (e.g. `AlertSoldier`) can chase it directly
        // rather than re-querying mid-think.  Only meaningful for
        // human actors with a door-pass / lift / building traversal
        // in flight; falls back to the live position for everyone
        // else, which is what `extract_forecast_input` returns and
        // `forecast_destination_for_ia` propagates.
        if matches!(
            entity,
            Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_)
        ) && let Some(input) = extract_forecast_input(entity)
        {
            let forecast = crate::ai::forecast_destination_for_ia(
                &input,
                doors_ref,
                &engine.fast_grid.level.sectors,
                &engine.fast_grid.level.sector_number_map,
            );
            view.forecasted_destination = forecast.position;
        }

        // AI handle == entity slot index (see `FighterSnapshot.handle =
        // target_id.index()` elsewhere, and `self.entities.get_mut(target as
        // usize)` for `CrossNpcAction` handlers).
        map.insert(entity_id.index(), view);
    }
    map
}

impl EngineInner {
    fn build_ai_sight_obstacles(
        &self,
        assets: &LevelAssets,
    ) -> crate::sight_obstacle::SharedSightObstacles {
        crate::sight_obstacle::SharedSightObstacles {
            static_obstacles: assets.static_sight_obstacles.clone(),
            dynamic_obstacles: std::sync::Arc::new(self.dynamic_sight_obstacles.clone()),
            static_active: std::sync::Arc::new(self.static_sight_obstacle_active.clone()),
        }
    }

    pub(crate) fn build_sim_scratch(&self, assets: &LevelAssets) -> SimScratch {
        let scratch = SimScratch {
            ai_entity_views: std::sync::Arc::new(build_entity_views(self)),
            ai_sight_obstacles: self.build_ai_sight_obstacles(assets),
        };
        crate::natives::set_script_sight_obstacles(scratch.ai_sight_obstacles.clone());
        scratch
    }

    /// Build a per-NPC [`AiPerTickData`] snapshot on demand, outside
    /// the main detection pass.
    ///
    /// The detection pass (see the builder at `engine/ai.rs:4319`)
    /// assembles a full-fidelity `AiPerTickData` with camp soldiers,
    /// nearby fighters, battle points, multiplicity, etc. — but it
    /// only runs once per frame per NPC.  Off-detection dispatch sites
    /// (timer events, reach-point events, panic, patrol, cross-NPC
    /// actions, civilian EventView...) previously called
    /// `AiPerTickData::stub()`, losing every field that matters for
    /// `battle_decisions` and swordfight tactics.  The symptom: a
    /// soldier with a valid `primary_target` but empty
    /// `enemy_sq_distances` bails to `return_to_duty`, producing the
    /// Reactiontime/Default ping-pong.
    ///
    /// This builder fills in everything that can be cheaply computed
    /// from the live entity store without re-running the detection
    /// loop: same-camp soldier snapshots for alert coordination,
    /// primary target metadata (position, posture, animation,
    /// carrier), `primary_target_is_pc`, friend-swap candidates for
    /// `ReconsiderEnemyApproach`, the avenger-on-the-roof wait
    /// position, and a single-target seed for
    /// `enemy_sq_distances` / `min_sq_enemy_distance` so
    /// `battle_decisions` doesn't see an empty list when a valid
    /// `primary_target` exists.  Fields that truly require the full
    /// detection scan (`nearby_fighters`, `us_battle_points`,
    /// `primary_target_multiplicity`,
    /// `unconscious_enemies`, `nearby_sleeping_enemies`,
    /// `primary_target_jump_line`, ...) remain empty — the same
    /// fidelity as the pre-existing timer-dispatch hand-roll at line
    /// 5927, now shared.
    ///
    /// Returns a stub for non-enemy-soldier entities (civilians, PCs,
    /// beggar/animal NPCs); their AI paths don't consult the combat
    /// tick fields, so the stub is adequate.
    pub(super) fn build_npc_tick_data(
        &self,
        npc_id: crate::element::EntityId,
        scratch: &SimScratch,
        assets: &LevelAssets,
    ) -> crate::ai::AiPerTickData {
        self.build_npc_tick_data_for_target(npc_id, scratch, assets, None)
    }

    pub(super) fn build_npc_tick_data_for_target(
        &self,
        npc_id: crate::element::EntityId,
        scratch: &SimScratch,
        assets: &LevelAssets,
        target_override: Option<crate::element::EntityId>,
    ) -> crate::ai::AiPerTickData {
        use crate::ai::AiPerTickData;

        // Pull the minimum we need from the NPC: its position, camp,
        // primary target handle, and the `couldnt_reachpoint` flag
        // (drives avenger-on-roof computation).
        let Some(entity) = self.entities.get(npc_id) else {
            return AiPerTickData::stub();
        };
        let Entity::Soldier(soldier) = entity else {
            // Non-enemy NPC — civilians use FriendlyAi which doesn't
            // consume combat tick fields.  Return stub.
            return AiPerTickData::stub();
        };
        let Some(ai) = soldier.npc.ai_brain.base() else {
            return AiPerTickData::stub();
        };
        let primary_target_handle = target_override
            .map(|id| id.index())
            .unwrap_or(ai.primary_target);
        let target_id = target_override.or_else(|| {
            (primary_target_handle != 0)
                .then(|| self.entity_id_for_index(primary_target_handle))
                .flatten()
        });
        let my_camp = soldier.soldier.cached_camp;
        let me_handle = ai.me;
        let me_pos = soldier.element.position_map();
        let couldnt_reachpoint = soldier
            .npc
            .ai_brain
            .enemy()
            .map(|e| e.base.couldnt_reachpoint)
            .unwrap_or(false);

        let mut tick = AiPerTickData::stub();
        tick.profile_manager = Some(assets.profile_manager.clone());
        tick.camp_soldiers = self.build_camp_soldier_tick_infos(npc_id, my_camp, scratch);
        // `fill_list_with_all_near_fighters` walks the global fighter
        // registry on every call.  Populate `nearby_fighters` here so
        // off-detection dispatch sites (timer events, reach-point
        // events, panic, patrol, cross-NPC actions, pending-stimuli
        // drain, …) see the same fighter view that the in-detection
        // builder produces.  Without this, AI predicates that consume
        // `tick.nearby_fighters` (rider charge target lookup,
        // PhalanxIsEncercledByEnemies, NumberOfNearbyArchersWhoNeedProtection,
        // ReconsiderPhalanx geometry, IsAnyFriendInThisPolygon)
        // observe an empty list outside swordfight substates.
        tick.nearby_fighters = self.build_nearby_fighters_for(npc_id, assets);

        // Phalanx right-chain "them" snapshots — consumed by
        // `PhalanxReinitializeThemList` so the leftmost member can
        // union each neighbour's enemies via the recursion.  Always
        // populated; empty when this NPC has no right neighbour.
        tick.phalanx_member_them_lists = self.build_phalanx_member_them_lists(npc_id);

        let Some(target_id) = target_id else {
            // No target selected — primary-target fields stay None,
            // enemy_sq_distances stays empty.  Friend-swap still
            // scans the other soldiers; the helper handles the
            // empty-target case.
            tick.friend_swap_candidates =
                build_friend_swap_candidates(&self.entities, npc_id, my_camp);
            return tick;
        };

        // Primary target metadata (position, posture, animation,
        // carrier) from the live entity store.
        let target_meta =
            lookup_primary_target_metadata(&self.entities, &self.sequence_manager, target_id);

        if let Some((pos, posture, anim, carrier_pos, carrier_handle)) = target_meta {
            tick.primary_target_position = Some(pos);
            tick.primary_target_posture = Some(posture);
            tick.primary_target_animation = anim;
            tick.primary_target_carrier_position = carrier_pos;
            tick.primary_target_carrier_handle = carrier_handle;

            // Seed enemy_sq_distances from the primary target so
            // `battle_decisions` sees a non-empty list when the
            // soldier has a valid target — same rationale as the
            // timer-dispatch seed at line 5916.
            let dx = pos.x - me_pos.x;
            let dy = (pos.y - me_pos.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let sq = (dx * dx + dy * dy) as i32;
            tick.enemy_sq_distances.push((target_id.index(), sq));
            tick.min_sq_enemy_distance = sq;
        }

        // primary_target_is_pc: look up the target's entity variant.
        tick.primary_target_is_pc = matches!(self.entities.get(target_id), Some(Entity::Pc(_)));

        if let Some(enemy_ai) = soldier.npc.ai_brain.enemy() {
            let my_company = enemy_ai.company_number;
            let my_pride = enemy_ai.soldier_profile_pride;
            tick.us_battle_points = 100 + my_pride as u32;

            let self_to_target_sq = tick.primary_target_position.map(|target_pos| {
                let dx = target_pos.x - me_pos.x;
                let dy =
                    (target_pos.y - me_pos.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
                dx * dx + dy * dy
            });

            for friend in &tick.nearby_fighters {
                if !friend.is_friendly || friend.handle == me_handle || !friend.is_able_to_fight {
                    continue;
                }

                if friend.is_pc {
                    tick.us_battle_points += 100;
                    if my_company > 0 {
                        tick.friends_lower_company = tick.friends_lower_company.saturating_add(1);
                    }
                    continue;
                }

                if !matches!(
                    friend.ai_state,
                    crate::ai::AiState::Default
                        | crate::ai::AiState::Wondering
                        | crate::ai::AiState::Seeking
                        | crate::ai::AiState::Attacking
                ) {
                    continue;
                }

                let friend_company = tick
                    .camp_soldiers
                    .iter()
                    .find(|cs| cs.handle == friend.handle)
                    .map(|cs| cs.company_number)
                    .unwrap_or(u16::MAX);
                if my_company > friend_company
                    && (ai.current_substate == crate::ai::Substate::AttackingReactiontime
                        || friend.ai_state == crate::ai::AiState::Attacking)
                {
                    tick.friends_lower_company = tick.friends_lower_company.saturating_add(1);
                }

                if my_pride > friend.soldier_profile_pride {
                    tick.soldiers_lower_pride = true;
                }
                tick.us_battle_points += 100 + friend.soldier_profile_pride as u32;

                if friend.rank == crate::profiles::ProfileRank::Soldier {
                    tick.simple_soldiers_near = true;
                }
                if friend.rank == crate::profiles::ProfileRank::Officer {
                    tick.has_officer_nearby = true;
                }

                if friend.ai_state == crate::ai::AiState::Attacking && friend.primary_target != 0 {
                    if crate::ai_enemy::is_any_swordfight_substate(friend.current_substate) {
                        tick.friends_nearer_to_enemy =
                            tick.friends_nearer_to_enemy.saturating_add(1);
                        if let Some((_, mult)) = tick
                            .primary_target_multiplicity
                            .iter_mut()
                            .find(|(h, _)| *h == friend.primary_target)
                        {
                            *mult = mult.saturating_add(1);
                        } else {
                            tick.primary_target_multiplicity
                                .push((friend.primary_target, 1));
                        }
                    } else if let Some(self_sq) = self_to_target_sq {
                        let Some(target_pos) = tick.primary_target_position else {
                            continue;
                        };
                        let dx = friend.position.x - target_pos.x;
                        let dy = (friend.position.y - target_pos.y)
                            * crate::position_interface::INVERSE_ASPECT_RATIO;
                        if dx * dx + dy * dy < self_sq {
                            tick.friends_nearer_to_enemy =
                                tick.friends_nearer_to_enemy.saturating_add(1);
                        }
                    }
                }
            }

            for &(attacker, target) in &self.ai_global.same_frame_target_claims {
                if attacker == me_handle || target == 0 {
                    continue;
                }
                let attacker_id = EntityId::Soldier(SoldierId(attacker));
                let Some(Entity::Soldier(s)) = self.entities.get(attacker_id) else {
                    continue;
                };
                if s.soldier.cached_camp != my_camp
                    || !s.element.active
                    || s.human.unconscious
                    || s.npc.life_points <= 0
                {
                    continue;
                }
                if target == primary_target_handle {
                    tick.friends_nearer_to_enemy = tick.friends_nearer_to_enemy.saturating_add(1);
                }
                if let Some((_, mult)) = tick
                    .primary_target_multiplicity
                    .iter_mut()
                    .find(|(h, _)| *h == target)
                {
                    *mult = mult.saturating_add(1);
                } else {
                    tick.primary_target_multiplicity.push((target, 1));
                }
            }
        }

        // Friend-swap candidates for ReconsiderEnemyApproach.
        tick.friend_swap_candidates = build_friend_swap_candidates(&self.entities, npc_id, my_camp);

        // Stashed-exit-door snapshot for the AlertSoldiers indoor
        // branch and the merry-man flee path.  Always populated
        // whenever the AI has stashed a door (irrespective of
        // in-building status), so paths that reach the door's
        // point_out through a sequence of substates still see the
        // cached geometry.  No fallback when no door is stashed.
        let stashed = soldier.npc.ai_brain.enemy().and_then(|e| e.my_door_index);
        if stashed.is_some() {
            let doors_slice: &[crate::gate::Door] = self
                .mission_script
                .as_ref()
                .and_then(|s| s.game_host())
                .map(|h| h.doors.as_slice())
                .unwrap_or(&[]);
            tick.my_exit_door = build_my_exit_door_info(stashed, doors_slice);
        }

        // Avenger-on-roof wait position — only computed when the AI
        // set the `couldnt_reachpoint` flag.
        if couldnt_reachpoint {
            let doors_slice: &[crate::gate::Door] = self
                .mission_script
                .as_ref()
                .and_then(|s| s.game_host())
                .map(|h| h.doors.as_slice())
                .unwrap_or(&[]);
            tick.avenger_on_roof_wait_position = precompute_avenger_on_roof_wait_position(
                &self.entities,
                doors_slice,
                npc_id,
                target_id,
                &|sector| self.get_sector_lift_type(sector),
            );
        }

        tick
    }

    fn build_camp_soldier_tick_infos(
        &self,
        npc_id: crate::element::EntityId,
        my_camp: crate::element::Camp,
        scratch: &SimScratch,
    ) -> Vec<crate::ai_enemy::CampSoldierInfo> {
        // Snapshot the ticking NPC (the brawler / self) once so each
        // officer's `is_detecting_cone` cache below evaluates
        // "officer is detecting brawler" against a single target.
        let me_brawler = self.entities.get(npc_id).and_then(|e| match e {
            Entity::Soldier(s) => Some(s),
            _ => None,
        });
        let me_brawler_data = me_brawler.map(|s| {
            let pos = s.element.position_map();
            (
                crate::coordinates::MapPoint::new(pos.x, pos.y),
                s.element.layer(),
                self.entity_data_inside_building(&s.element),
            )
        });
        let obstacles_owned = scratch.ai_sight_obstacles.clone();
        let obstacles = obstacles_owned.list();

        let mut camp_soldiers =
            Vec::with_capacity(self.entities.soldiers().count().saturating_sub(1));
        for (other_id, s) in self.entities.soldiers() {
            if other_id == npc_id {
                continue;
            }
            if s.soldier.cached_camp != my_camp || !s.element.active || s.human.unconscious {
                continue;
            }
            let able_to_fight = s.element.active && !s.human.unconscious && s.npc.life_points > 0;
            let Some(enemy_ai) = s.npc.ai_brain.enemy() else {
                continue;
            };
            let in_building = self.entity_data_inside_building(&s.element);
            let forecast_destination = {
                let doors = self
                    .mission_script
                    .as_ref()
                    .and_then(|ms| ms.game_host())
                    .map(|h| h.doors.as_slice())
                    .unwrap_or(&[]);
                let pos_now = s.element.position_map();
                let door_pass = s
                    .actor
                    .active_door_pass
                    .as_ref()
                    .map(|dp| (dp.door_index, dp.direct));
                let input = crate::ai::ForecastInput {
                    position_map_x: pos_now.x,
                    position_map_y: pos_now.y,
                    sector: s.element.sector().map(u16::from).unwrap_or(0),
                    layer: s.element.layer(),
                    direction: s.element.direction() as u16,
                    forecasted_movement_z: s
                        .element
                        .sprite
                        .position_iface
                        .get_forecasted_movement()
                        .z,
                    door_pass,
                };
                crate::ai::forecast_destination_for_ia(
                    &input,
                    doors,
                    &self.fast_grid.level.sectors,
                    &self.fast_grid.level.sector_number_map,
                )
                .position
            };
            let position = s.element.position_map();
            // Snapshot the soldier's `DETECTABLE_BODY` list — handles of
            // corpses they have not yet reacted to.  Snapshotting the
            // data here lets AI predicates run off `tick.camp_soldiers`
            // alone instead of poking at the live detectable list.
            let detectable_body_idx = crate::element::DetectableType::Body as usize;
            let detectable_bodies = s
                .npc
                .detectable_lists
                .get(detectable_body_idx)
                .map(|list| {
                    let mut bodies = Vec::with_capacity(list.len());
                    bodies.extend(list.iter().filter_map(|d| d.element.map(|e| e.index())));
                    bodies
                })
                .unwrap_or_default();
            let cs_position = crate::ai::Position {
                x: position.x,
                y: position.y,
                sector: s.element.sector(),
                level: s.element.layer(),
            };
            // Snapshot "officer is detecting brawler" (full radius +
            // cone + opaque-LOS) so `MaybeOfficerSeesMeFighting`'s
            // ≥350² band reads a cached flag instead of redoing the
            // geometry per fighter pair.  Short-circuit when the
            // viewer is blind / indoors / KO'd or when the target sits
            // inside a building; fold those into the cached `false` here.
            let eye_blind = s.npc.eye_status.is_blind();
            let is_detecting_cone = match me_brawler_data {
                Some((me_pos, me_layer, me_in_building))
                    if !eye_blind && !in_building && able_to_fight && !me_in_building =>
                {
                    let viewer = crate::coordinates::MapPoint::new(position.x, position.y);
                    crate::ai_vision::is_detecting_target(
                        viewer,
                        s.element.direction(),
                        (s.npc.view_direction[0], s.npc.view_direction[1]),
                        s.npc.real_half_aperture,
                        s.npc.view_radius,
                        me_pos,
                        me_layer,
                        obstacles,
                        &self.fast_grid,
                    )
                }
                _ => false,
            };
            camp_soldiers.push(crate::ai_enemy::CampSoldierInfo {
                handle: other_id.index(),
                position: cs_position,
                direction: s.element.direction() as u16,
                rank: enemy_ai.soldier_profile_rank,
                ai_state: s.npc.ai_state(),
                ai_substate: s.npc.ai_substate(),
                is_able_to_fight: able_to_fight,
                is_able_to_help: crate::ai_enemy::soldier_is_able_to_help_state(
                    able_to_fight,
                    s.npc.ai_state(),
                    s.npc.ai_substate(),
                ),
                script_locked: enemy_ai.base.script_locked,
                layer: s.element.layer(),
                report_type: enemy_ai.base.my_reconnaissance_report.report_type,
                report_seek_position: enemy_ai.base.my_reconnaissance_report.seek_position,
                report_seen_bodies: enemy_ai.base.my_reconnaissance_report.seen_bodies.clone(),
                report_charly: enemy_ai.base.my_reconnaissance_report.charly,
                alert_soldiers_point: enemy_ai.base.alert_soldiers_point,
                patrol_chief: enemy_ai.base.patrol_chief,
                antagonist: enemy_ai.base.antagonist,
                duty_flag: enemy_ai.soldier_profile_duty,
                is_tower_guard: enemy_ai.tower_guard,
                company_number: enemy_ai.company_number,
                in_building,
                forecast_destination,
                detectable_bodies,
                seek_position: enemy_ai.base.seek_position,
                current_task_priority: enemy_ai.current_task_priority,
                minimal_task_priority: enemy_ai.minimal_task_priority,
                view_direction: s.npc.view_direction,
                view_radius: s.npc.view_radius,
                real_half_aperture: s.npc.real_half_aperture,
                eye_blind,
                is_detecting_cone,
            });
        }
        camp_soldiers
    }

    /// Build a `nearby_fighters` snapshot list for one enemy NPC.
    ///
    /// Walks the entity store directly, the same scan-the-global-fighter-
    /// registry approach used by swordfight reconsideration. Filters
    /// non-self entries to the same 500-unit Chebyshev radius the
    /// detection-pass builder applies.
    ///
    /// Returns an empty Vec for non-enemy soldiers — civilians and
    /// PCs don't consume `nearby_fighters`.
    pub(super) fn build_nearby_fighters_for(
        &self,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) -> Vec<crate::ai_enemy::FighterSnapshot> {
        use crate::ai::Position;
        use crate::ai_enemy::FighterSnapshot;
        use crate::element::Posture;

        let Some(Entity::Soldier(soldier)) = self.entities.get(npc_id) else {
            return Vec::new();
        };
        let Some(enemy_ai) = soldier.npc.ai_brain.enemy() else {
            return Vec::new();
        };
        let me_pos_pt = soldier.element.position_map();
        let my_layer = soldier.element.layer();
        let my_camp = soldier.soldier.cached_camp;
        let me_handle = enemy_ai.base.me;

        const SWORDFIGHT_RADIUS: f32 = 500.0;

        // Build a friendly soldier snapshot for `handle` (which may be self).
        let build_soldier = |handle: u32| -> Option<FighterSnapshot> {
            let s = self.entities.get_soldier(SoldierId(handle))?;
            if !s.element.active || s.human.unconscious || s.npc.life_points <= 0 {
                // Detection-pass builder only feeds soldier_snapshots
                // through the alive+conscious filter (see
                // `snapshots.rs`).  Apply the same filter here.
                return None;
            }
            let pos = s.element.position_map();
            let enemy_ai_other = s
                .npc
                .ai_brain
                .enemy()
                .unwrap_or_else(|| panic!("active soldier {handle} has no EnemyAi brain"));
            let soldier_profile = assets
                .profile_manager
                .get_soldier(s.soldier.soldier_profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "soldier {handle} requires missing soldier profile {}",
                        u32::from(s.soldier.soldier_profile_index)
                    )
                });
            let has_formation = soldier_profile.formation;
            let fighting_ability = {
                let base = soldier_profile.fighting;
                if s.soldier.cached_camp == Camp::Lacklandists {
                    let diff = crate::player_profile::DifficultyLevel::current();
                    diff.modify_capacity(
                        base,
                        crate::player_profile::difficulty_params::EASY_ENEMY_FIGHTING,
                        crate::player_profile::difficulty_params::HARD_ENEMY_FIGHTING,
                        100,
                    )
                } else {
                    base
                }
            };
            let bow_profile = if soldier_profile.shooting_weapon_id == 0 {
                None
            } else {
                Some(
                    assets
                        .profile_manager
                        .get_bow(soldier_profile.shooting_weapon_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "soldier {handle} requires missing bow profile {}",
                                soldier_profile.shooting_weapon_id
                            )
                        }),
                )
            };
            let is_archer_unit = bow_profile
                .map(|bow| bow.normal_shoot.range > 0)
                .unwrap_or(false);
            let bow_max_range = bow_profile
                .map(|bow| {
                    if bow.has_long_shoot {
                        bow.long_shoot.range
                    } else {
                        bow.normal_shoot.range
                    }
                })
                .unwrap_or(0);
            let hth_id = enemy_ai_other.hth_weapon_id;
            let hth_profile = assets
                .profile_manager
                .get_hth_weapon(hth_id)
                .unwrap_or_else(|| {
                    panic!("soldier {handle} requires missing HtH weapon profile {hth_id}")
                });
            let (sword_range_default, sword_range_maximal, sword_range_uber) = (
                hth_profile.distance[crate::weapons::WeaponDistance::Default as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Maximal as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Uber as usize],
            );
            let weapon_is_shield = hth_profile.shield;
            let has_shield_anim = s
                .element
                .sprite
                .has_animation(crate::order::OrderType::WaitingShield);
            let is_shield_bearer = weapon_is_shield && has_shield_anim;
            let in_recovery = matches!(
                s.actor.old_action,
                crate::order::OrderType::BeingHitSword
                    | crate::order::OrderType::ExtractingArrowSword
                    | crate::order::OrderType::DyingSword
                    | crate::order::OrderType::BeingDeadSword
                    | crate::order::OrderType::FallingBackSword
                    | crate::order::OrderType::BeingUnconsciousSword
                    | crate::order::OrderType::BeingDeadFallenBackSword
                    | crate::order::OrderType::StandingUpSword
            );
            let seek_position = Position {
                x: enemy_ai_other.base.seek_position.x,
                y: enemy_ai_other.base.seek_position.y,
                sector: enemy_ai_other.base.seek_position.sector,
                level: s.element.layer(),
            };
            let opponent_handles: Vec<u32> =
                s.human.opponents.iter().map(|id| id.index()).collect();
            let number_of_opponents = opponent_handles.len().min(u16::MAX as usize) as u16;
            let is_friendly = s.soldier.cached_camp == my_camp;
            Some(FighterSnapshot {
                handle,
                position: Position {
                    x: pos.x,
                    y: pos.y,
                    sector: None,
                    level: s.element.layer(),
                },
                direction: s.element.direction() as u16,
                is_friendly,
                is_swordfighting: !s.human.opponents.is_empty(),
                is_able_to_fight: true, // filtered above
                is_tied: s.element.posture == Posture::Tied,
                is_unconscious: false, // filtered above
                is_dead: false,        // filtered above
                is_carried: false,
                is_pc: false,
                is_soldier: true,
                rank: enemy_ai_other.soldier_profile_rank,
                primary_target: enemy_ai_other.base.primary_target,
                principal_opponent: s.human.opponents.first().map(|id| id.index()).unwrap_or(0),
                number_of_opponents,
                opponent_handles,
                sword_range_default,
                sword_range_maximal,
                sword_range_uber,
                fighting_ability,
                has_formation,
                is_shield_bearer,
                is_archer_unit,
                is_tower_guard: enemy_ai_other.tower_guard,
                is_vip: soldier_profile.vip,
                soldier_profile_pride: enemy_ai_other.soldier_profile_pride,
                is_robin: false,
                left_combat_neighbour: enemy_ai_other.left_combat_neighbour,
                right_combat_neighbour: enemy_ai_other.right_combat_neighbour,
                is_in_recovery_animation: in_recovery,
                in_sword_action_state: s.actor.action_state.is_sword(),
                elevation: s.element.position().z as u16,
                seek_position,
                current_substate: s.npc.ai_substate() as u32,
                archer_behind_me: enemy_ai_other.archer_behind_me,
                ai_state: s.npc.ai_state(),
                shield_bearer_before_me: enemy_ai_other.shield_bearer_before_me,
                hth_weapon_id: hth_id,
                action_state: s.actor.action_state,
                shield_bearer_direction: enemy_ai_other.shield_bearer_direction,
                shield_bearer_seek_position: seek_position,
                bow_max_range,
            })
        };

        // Build an enemy PC snapshot for `handle`.
        let build_pc = |handle: u32| -> Option<FighterSnapshot> {
            let pc = self.entities.get_pc(PcId(handle))?;
            if !pc.element.active || pc.pc.life_points <= 0 {
                return None;
            }
            let is_unconscious = pc.human.unconscious;
            let is_carried = pc.human.carrier.is_some();
            let alive = !is_unconscious;
            let pos = pc.element.position_map();
            let character = assets
                .profile_manager
                .get_character(pc.pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "PC {handle} requires missing character profile {}",
                        u32::from(pc.pc.profile_index)
                    )
                });
            let hth_id = character.hth_weapon_id;
            let fighting_ability = character.fighting;
            let hth_profile = assets
                .profile_manager
                .get_hth_weapon(hth_id)
                .unwrap_or_else(|| {
                    panic!("PC {handle} requires missing HtH weapon profile {hth_id}")
                });
            let (sword_range_default, sword_range_maximal, sword_range_uber) = (
                hth_profile.distance[crate::weapons::WeaponDistance::Default as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Maximal as usize],
                hth_profile.distance[crate::weapons::WeaponDistance::Uber as usize],
            );
            let in_recovery = !alive
                || matches!(
                    pc.actor.old_action,
                    crate::order::OrderType::BeingHitSword
                        | crate::order::OrderType::ExtractingArrowSword
                        | crate::order::OrderType::DyingSword
                        | crate::order::OrderType::BeingDeadSword
                        | crate::order::OrderType::FallingBackSword
                        | crate::order::OrderType::BeingUnconsciousSword
                        | crate::order::OrderType::BeingDeadFallenBackSword
                        | crate::order::OrderType::StandingUpSword
                );
            let opponent_handles: Vec<u32> =
                pc.human.opponents.iter().map(|id| id.index()).collect();
            let number_of_opponents = opponent_handles.len().min(u16::MAX as usize) as u16;
            let pc_seek_position = Position {
                x: pos.x,
                y: pos.y,
                sector: None,
                level: pc.element.layer(),
            };
            Some(FighterSnapshot {
                handle,
                position: Position {
                    x: pos.x,
                    y: pos.y,
                    sector: None,
                    level: pc.element.layer(),
                },
                direction: pc.element.direction() as u16,
                is_friendly: false,
                is_swordfighting: !pc.human.opponents.is_empty(),
                is_able_to_fight: alive
                    && !matches!(pc.element.posture, Posture::Tree | Posture::Spy),
                is_tied: pc.element.posture == Posture::Tied,
                is_unconscious,
                is_dead: false, // filtered life_points > 0 above
                is_carried,
                is_pc: true,
                is_soldier: false,
                rank: crate::profiles::ProfileRank::None,
                primary_target: pc.pc.melee_target.map(|id| id.index()).unwrap_or(0),
                principal_opponent: pc.human.opponents.first().map(|id| id.index()).unwrap_or(0),
                number_of_opponents,
                opponent_handles,
                sword_range_default,
                sword_range_maximal,
                sword_range_uber,
                fighting_ability,
                has_formation: false,
                is_shield_bearer: false,
                is_archer_unit: false,
                is_tower_guard: false,
                is_vip: character.vip,
                soldier_profile_pride: 0,
                is_robin: pc.pc.robin,
                left_combat_neighbour: 0,
                right_combat_neighbour: 0,
                is_in_recovery_animation: in_recovery,
                in_sword_action_state: pc.actor.action_state.is_sword(),
                elevation: pc.element.sprite.position_iface.get_elevation() as u16,
                seek_position: pc_seek_position,
                current_substate: 0,
                archer_behind_me: 0,
                ai_state: crate::ai::AiState::default(),
                shield_bearer_before_me: 0,
                hth_weapon_id: hth_id,
                action_state: pc.actor.action_state,
                shield_bearer_direction: 0,
                shield_bearer_seek_position: pc_seek_position,
                bow_max_range: 0,
            })
        };

        let mut out: Vec<FighterSnapshot> = Vec::with_capacity(1 + self.pc_ids.len() + 4);

        // Self entry first — no radius filter (the AI is at distance 0).
        out.push(build_soldier(me_handle).unwrap_or_else(|| {
            panic!("enemy AI self {me_handle} is absent from the live fighter registry")
        }));

        // All live soldiers in the same combat radius. Scan the global
        // camp fighter registries when rebuilding the us/them lists;
        // using the persisted per-AI lists here made combat-position
        // cleanup blind to same-camp fighters and allowed dogpiles.
        for (other_id, s) in self.entities.soldiers() {
            if other_id.index() == me_handle {
                continue;
            }
            if s.element.layer() != my_layer {
                continue;
            }
            let p = s.element.position_map();
            let dx = p.x - me_pos_pt.x;
            let dy = (p.y - me_pos_pt.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            if dx.abs().max(dy.abs()) > SWORDFIGHT_RADIUS {
                continue;
            }
            if let Some(snap) = build_soldier(other_id.index()) {
                out.push(snap);
            }
        }

        // PCs are royalist fighters from the enemy AI's perspective.
        if my_camp != Camp::Royalists {
            for &pc_id in &self.pc_ids {
                let Some(Entity::Pc(pc)) = self.entities.get(pc_id) else {
                    continue;
                };
                if pc.element.layer() != my_layer {
                    continue;
                }
                let p = pc.element.position_map();
                let dx = p.x - me_pos_pt.x;
                let dy = (p.y - me_pos_pt.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
                if dx.abs().max(dy.abs()) > SWORDFIGHT_RADIUS {
                    continue;
                }
                if let Some(snap) = build_pc(pc_id.index()) {
                    out.push(snap);
                }
            }
        }

        out
    }

    /// Snapshot every right-chain phalanx member's `list_them` (plus
    /// position/direction) so `PhalanxReinitializeThemList` can union
    /// each neighbour's enemies into the shared list without mutating
    /// sibling AI brains.  The chain is walked via depth-first recursion
    /// through `right_combat_neighbour`; we materialise the chain
    /// up-front because Rust's borrow rules forbid mutable cross-NPC
    /// state during a single AI tick.
    pub(super) fn build_phalanx_member_them_lists(
        &self,
        npc_id: crate::element::EntityId,
    ) -> Vec<crate::ai::PhalanxMemberThemList> {
        use crate::ai::{PhalanxMemberThemList, Position};
        let Some(Entity::Soldier(soldier)) = self.entities.get(npc_id) else {
            return Vec::new();
        };
        let Some(enemy_ai) = soldier.npc.ai_brain.enemy() else {
            return Vec::new();
        };

        let mut out: Vec<PhalanxMemberThemList> = Vec::new();
        let mut current = enemy_ai.right_combat_neighbour;
        // Cap at 16 like the consumer's right-chain walk; phalanxes are
        // small and the cap guards against any cycle in cached neighbour
        // links.
        for _ in 0..16 {
            if current == 0 {
                break;
            }
            let Some(s) = self.entities.get_soldier(SoldierId(current)) else {
                break;
            };
            if !s.element.active || s.human.unconscious || s.npc.life_points <= 0 {
                break;
            }
            let Some(neighbour_ai) = s.npc.ai_brain.enemy() else {
                break;
            };
            let pos = s.element.position_map();
            out.push(PhalanxMemberThemList {
                handle: current,
                current_them_list: neighbour_ai.list_them.clone(),
                position: Position {
                    x: pos.x,
                    y: pos.y,
                    sector: None,
                    level: s.element.layer(),
                },
                direction: s.element.direction() as u16,
            });
            let next = neighbour_ai.right_combat_neighbour;
            if next == 0 || next == current {
                break;
            }
            current = next;
        }
        out
    }
}

impl EngineInner {
    // ─── AI initialization ──────────────────────────────────────

    /// Initialize AI for all NPCs and reset global AI state.
    ///
    /// Called from `initialize()` after level loading, and again after
    /// deserialization when re-initialization is requested.
    pub(crate) fn init_ai(&mut self, assets: &mut LevelAssets) {
        // Reset global AI state
        // think-method recursion depth = 0
        self.ai_global.there_are_royalist_soldiers = false;
        self.ai_global.there_are_lacklandist_soldiers = false;
        self.ai_global.overall_alert_status = crate::ai::AlertLevel::Green;
        self.ai_global.overall_villain_alert_status = crate::ai::AlertLevel::Green;
        self.ai_global.init_green_yellow_red_alert_soldiers();

        // golden_eye_mode is set from CliArgs after initialize() returns

        // Build the houses list and door rally points.  Collects every
        // building sector, attaches its doors, records occupants, and
        // anchors a rally point outside each door at
        // `AI_DOOR_RALLY_POINT_DISTANCE`.  Must run before the NPC init
        // loop below, because `InitOneAI` reads `leave_house_number`
        // off the AI controller which is assigned here.
        self.initialize_buildings();

        // Beam hiking-path waypoints that sit just outside a building
        // door into the building's interior.  Mutates the shared
        // `hiking_paths` arc in place through `Arc::make_mut` so
        // subsequent NPC clones see the beamed paths.
        {
            let paths = std::sync::Arc::make_mut(&mut assets.hiking_paths);
            for path in paths.iter_mut() {
                for wp in path.waypoints.iter_mut() {
                    for door in &self.ai_global.door_seek_infos {
                        if door.door_type != crate::gate::DoorType::Building {
                            continue;
                        }
                        // Chebyshev distance <= 5.
                        let dx = (wp.x as f32 - door.point_out.x).abs();
                        let dy = (wp.y as f32 - door.point_out.y).abs();
                        if dx.max(dy) <= 5.0 {
                            wp.x = door.position_in.x as i16;
                            wp.y = door.position_in.y as i16;
                            wp.sector = door.position_in.sector.map(u16::from).unwrap_or(0);
                            wp.level = door.position_in.level;
                            break;
                        }
                    }
                }
            }
        }

        // Teleport standalone seek points (those used by AI
        // investigators) that sit just outside a building door to
        // the door's inside position — same rule as the waypoint
        // beaming above.  Already implemented as
        // `AiGlobalState::teleport_seek_points_inside_doors`.
        self.ai_global.teleport_seek_points_inside_doors();

        // Initialize each NPC's AI.
        let npc_ids: Vec<EntityId> = self.entities.npc_ids().collect();
        let hiking_paths = assets.hiking_paths.clone();
        // Populate the handle → entity view map so the per-NPC
        // init_ctx hands each AI a usable map (even though init
        // mostly just reads self position).
        let scratch = self.build_sim_scratch(assets);
        // For "get soldier from all by id" in the AI tick: copy the
        // level's soldier load-order array onto AiGlobalState so
        // AiContext can resolve script-baked friend IDs.
        self.ai_global.all_soldier_handles = std::sync::Arc::new(
            assets
                .all_soldier_entity_ids
                .iter()
                .map(|eid| eid.index())
                .collect(),
        );
        let entity_views = scratch.ai_entity_views.clone();
        let sight_obstacles = scratch.ai_sight_obstacles.clone();
        let all_soldier_handles = self.ai_global.all_soldier_handles.clone();

        // Snapshot of every live human in the engine; every per-NPC
        // init pass reuses the same list to build its detectable enemy
        // array.  Equivalent to iterating the engine's element list
        // inside each per-NPC init.
        let potential_detectables = build_potential_detectables(self);
        let ambush_points_count = self.ai_global.ambush_points.len();

        let all_soldier_entity_ids = assets.all_soldier_entity_ids.clone();
        let soldier_subordinate_ids = assets.soldier_subordinate_ids.clone();
        let fast_grid = self.fast_grid.clone();
        for &npc_id in &npc_ids {
            self.init_one_ai(
                npc_id,
                &hiking_paths,
                &potential_detectables,
                ambush_points_count,
                &entity_views,
                &sight_obstacles,
                &fast_grid,
                &all_soldier_handles,
                &all_soldier_entity_ids,
                &soldier_subordinate_ids,
            );
        }

        // Lift each ambush point's 2D position into 3D (eye height
        // = 32 units above the ground) and assign a sequential ID.
        // The 3D anchor feeds the sight-polygon query that decides
        // whether an NPC on the ambush point can be seen; the ID is
        // how AI scripts reference the point.
        for (idx, ap) in self.ai_global.ambush_points.iter_mut().enumerate() {
            ap.position_3d = crate::coordinates::WorldPoint3D {
                x: ap.position.x,
                y: ap.position.y,
                z: 32.0,
            };
            ap.id = idx as u16;
        }

        tracing::info!("AI initialized for {} NPCs", npc_ids.len(),);
    }

    /// Per-NPC initialization pass — runs the per-NPC init for both
    /// enemy and friendly AI.
    ///
    /// Runs every entity-level side effect that must happen once at
    /// level load:
    ///
    /// 1. `InitializeDirectionOffsetVeryOld` — seed `direction_old`
    ///    from the current body direction so the vision pipeline has a
    ///    stable starting value.
    /// 2. `InitViewRadius` — clamp `view_radius` / `view_radius_base`
    ///    / `view_radius_goal` to the engine's standard view radius
    ///    for this level (day/night dependent).
    /// 3. Give Merry Man archers in forest levels their starting bow
    ///    ammo (`MERRY_MAN_ARROWS`).
    /// 4. Build the per-NPC "detectable enemies" list from a snapshot
    ///    of live humans.
    /// 5. Stuck-in-obstacle correction (Malignity only): if the NPC
    ///    starts inside a motion obstacle, push its move box out to
    ///    an authorized position and rewrite its map position.
    /// 6. `StoreInitialPositionParameters` — freeze the NPC's current
    ///    position / sector / level / facing as the "initial" values
    ///    that the AI returns to after idle wanders.
    /// 7. Initialize this NPC's patrol path from `path_id`, then run
    ///    `TestIfPathIsFine` against the fast-find grid; clear the
    ///    patrol if any segment intersects an obstacle.
    /// 8. Seed `old_life_points` / `initial_life_points` on enemy AIs
    ///    for the "still has his initial HP" check.  Difficulty-based
    ///    life-point scaling is already applied at entity-spawn time
    ///    in `engine::level_loading::spawn_soldier`, so we just
    ///    snapshot the current value here.
    /// 9. Fill this enemy's `ambush_point_status` vector with
    ///    `Far` × `ambush_points_count` so `RefreshAmbushPoints`
    ///    has a slot per global ambush point.
    /// 10. Dispatch to the subclass's `init_one_ai` for the
    ///     initial-action / state-transition / return-to-duty logic.
    #[allow(clippy::too_many_arguments)]
    fn init_one_ai(
        &mut self,
        npc_id: EntityId,
        hiking_paths: &std::sync::Arc<Vec<crate::level_data::RawHikingPath>>,
        potential_detectables: &[PotentialDetectable],
        ambush_points_count: usize,
        entity_views: &SharedAiEntityViews,
        sight_obstacles: &crate::sight_obstacle::SharedSightObstacles,
        fast_grid: &crate::fast_find_grid::FastFindGrid,
        all_soldier_handles: &std::sync::Arc<Vec<u32>>,
        all_soldier_entity_ids: &[EntityId],
        soldier_subordinate_ids: &[Vec<u16>],
    ) {
        // -- Phase 1: Peek at the entity to classify (enemy / friendly,
        //    camp) and read the fields we need for the obstacle fix. --
        let (is_enemy, is_friendly, self_camp, pos_map, layer, move_box_opt) = {
            let Some(entity) = self.entities.get(npc_id) else {
                return;
            };
            let (is_enemy, is_friendly, self_camp) = match entity {
                Entity::Soldier(s) => (
                    s.npc.ai_brain.enemy().is_some(),
                    false,
                    s.soldier.cached_camp,
                ),
                Entity::Civilian(c) => (
                    false,
                    c.npc.ai_brain.friendly().is_some(),
                    c.civilian.cached_camp,
                ),
                _ => return,
            };
            let elem = entity.element_data();
            let move_box = entity
                .actor_data()
                .map(|_| *entity.position_iface().get_move_box());
            (
                is_enemy,
                is_friendly,
                self_camp,
                elem.position_map(),
                elem.layer(),
                move_box,
            )
        };
        if !(is_enemy || is_friendly) {
            return;
        }

        // -- Phase 2: Stuck-in-obstacle correction (enemy only). --
        // If the NPC's move-box overlaps the playable area, attempt to
        // push it to an authorized position via `find_authorized_position`.
        if is_enemy && let Some(move_box) = move_box_opt {
            let mut abs_box = move_box.translated(pos_map);
            if !self.fast_grid.is_position_authorized(&abs_box, layer)
                && self.fast_grid.find_authorized_position(&mut abs_box, layer)
            {
                let new_center = abs_box.center();
                if let Some(entity) = self.entities.get_mut(npc_id)
                    && entity.actor_data().is_some()
                {
                    let new_center_map = new_center;
                    let pi = entity.position_iface_mut();
                    pi.set_map_position(new_center_map);
                    entity.element_data_mut().set_position_map(new_center_map);
                }
            }
        }

        // -- Phase 3: Build the detectable-enemy list for this NPC. --
        let detectables =
            build_detectable_enemies_for(self_camp, is_friendly, npc_id, potential_detectables);

        // -- Phase 4: Re-read entity (post-fix) and mutate all the
        //    per-NPC state fields in one shot. --
        let standard_view_radius = if self.standard_view_polygon_radius > 0 {
            self.standard_view_polygon_radius
        } else {
            ai_vision::DEFAULT_VIEW_RADIUS
        };
        let is_forest_level = self.weather.is_forest_level;

        // `entity_building_sector` needs a `&self` borrow; compute it
        // up-front while we don't hold a mutable entity borrow.
        let building_sector = {
            let Some(entity) = self.entities.get(npc_id) else {
                return;
            };
            self.entity_building_sector(entity.element_data().sector())
        };

        // Determine whether this NPC is a Merry-Man archer (Royalist
        // soldier, forest level, archer flag set by the level loader).
        let is_merry_man_archer = if is_enemy {
            let Some(entity) = self.entities.get(npc_id) else {
                return;
            };
            let is_archer = entity.enemy_ai().map(|e| e.is_archer()).unwrap_or(false);
            let is_rider = entity.soldier_data().map(|s| s.rider).unwrap_or(false);
            self_camp == Camp::Royalists && is_forest_level && is_archer && !is_rider
        } else {
            false
        };

        // Grab the (possibly corrected) map position / direction /
        // sector / layer before the write-back borrow.
        let (pos_map_final, direction_final, sector_final, layer_final, current_lp) = {
            let Some(entity) = self.entities.get(npc_id) else {
                return;
            };
            let elem = entity.element_data();
            let lp = entity.npc_data().map(|n| n.life_points).unwrap_or(0);
            (
                elem.position_map(),
                elem.direction(),
                elem.sector(),
                elem.layer(),
                lp,
            )
        };

        // Write-back block: mutate every field this init pass owns.
        {
            let Some(entity) = self.entities.get_mut(npc_id) else {
                return;
            };
            if let Some(npc) = entity.npc_data_mut() {
                // `initialize_direction_offset_very_old`: seed from current body dir.
                npc.direction_old = direction_final;

                // `init_view_radius`: real radius + goal = standard view
                // radius.  We also seed `view_radius_base` so subsequent
                // alert/drunk modifiers scale off the correct baseline.
                npc.view_radius = standard_view_radius;
                npc.view_radius_base = standard_view_radius;
                npc.view_radius_goal = standard_view_radius;

                if is_merry_man_archer {
                    // Seed the bow ammo for forest-level Merry Man archers.
                    npc.number_of_arrows = MERRY_MAN_ARROWS;
                }

                // Detectable enemies list (`DetectableType::Enemy`
                // slot).  Other slots (Body/Object/Friend/...) are
                // populated later by runtime events.
                let enemy_idx = DetectableType::Enemy as usize;
                npc.detectable_lists[enemy_idx] = detectables;

                // `store_initial_position_parameters`: snapshot current
                // position, sector, level, and facing into the
                // initial-position fields.
                npc.initial_position_x = pos_map_final.x;
                npc.initial_position_y = pos_map_final.y;
                npc.initial_position_sector = sector_final;
                npc.initial_position_level = layer_final;
                let dir_vec = crate::shadow_polygon::sector_to_direction(direction_final);
                npc.initial_view_direction.x = dir_vec[0];
                npc.initial_view_direction.y = dir_vec[1];
            }

            // Switch on the NPC's camp to set the static camp-present
            // flags (`there_are_royalist_soldiers` /
            // `there_are_lacklandist_soldiers`).  Reading
            // `npcs_can_be_enemies()` later gates mixed-camp soldier
            // hostility on both flags being true.  The life-point
            // Easy/Hard scaling on the Lacklandist arm is already
            // applied at spawn time in `level_loading::spawn_soldier`.
            if is_enemy {
                match self_camp {
                    Camp::Royalists => self.ai_global.there_are_royalist_soldiers = true,
                    Camp::Lacklandists => self.ai_global.there_are_lacklandist_soldiers = true,
                    _ => {}
                }
            }

            // Enemy-specific state.
            if is_enemy && let Some(enemy) = entity.enemy_ai_mut() {
                // `old_life_points` = `initial_life_points` =
                // `get_life_points()`.  The level loader already applied
                // difficulty scaling to `cached_max_life_points` at
                // `engine::level_loading::spawn_soldier`, so the current
                // life points are already correct.
                let clamped = current_lp.clamp(0, 255) as u8;
                enemy.old_life_points = clamped;
                enemy.initial_life_points = clamped;

                // Reset the ambush-point-status array and insert
                // `AMBUSH_POINT_FAR` for every point in the global
                // ambush array.
                enemy.ambush_point_array_reset = true;
                enemy.ambush_point_status.clear();
                enemy
                    .ambush_point_status
                    .resize(ambush_points_count, crate::ai_enemy::AmbushPointStatus::Far);
            }
        }

        // -- Phase 5: Patrol path init + TestIfPathIsFine. --
        // Initialize the path from path_id, then test it; on failure,
        // assert in debug and silently clear in release.
        let patrol_path_opt = {
            let Some(entity) = self.entities.get(npc_id) else {
                return;
            };
            entity
                .ai_controller()
                .and_then(|ai| ai.path_id)
                .and_then(|pid| crate::ai::PatrolPath::new(pid, hiking_paths))
        };

        let patrol_path_ok = if let Some(ref patrol) = patrol_path_opt {
            // Grab the actual hiking-path waypoints + the NPC's move
            // box and run the obstacle check.
            let waypoints = hiking_paths
                .get(usize::from(patrol.hiking_path_index))
                .map(|p| p.waypoints.as_slice())
                .unwrap_or(&[]);
            let move_box = move_box_opt.unwrap_or_default();
            let ok = test_hiking_path_fine(&self.fast_grid, waypoints, &move_box);
            if !ok {
                tracing::warn!(
                    npc = npc_id.index(),
                    path_id = patrol.hiking_path_index.get(),
                    waypoints = waypoints.len(),
                    move_box = ?move_box,
                    "BUG: patrol path rejected by TestIfPathIsFine — debug asserts this \
                     never fails; in release the path is cleared and the NPC silently \
                     stops patrolling"
                );
            }
            ok
        } else {
            false
        };

        // -- Phase 6: Build the init ctx and commit patrol/path state. --
        let init_ctx = {
            let Some(entity) = self.entities.get(npc_id) else {
                return;
            };
            build_ai_context_from_entity(
                entity,
                0,
                building_sector,
                is_forest_level,
                standard_view_radius,
                entity_views,
                sight_obstacles,
                fast_grid,
                hiking_paths,
                all_soldier_handles,
            )
        };

        {
            let Some(entity) = self.entities.get_mut(npc_id) else {
                return;
            };
            if let Some(ai) = entity.ai_controller_mut() {
                ai.initial_position = crate::ai::Position {
                    x: pos_map_final.x,
                    y: pos_map_final.y,
                    sector: sector_final,
                    level: layer_final,
                };
                ai.initial_view_direction = direction_final.rem_euclid(16) as u16;
                if patrol_path_opt.is_some() && patrol_path_ok {
                    ai.patrol_path = patrol_path_opt;
                    ai.has_patrol_path = true;
                } else {
                    ai.patrol_path = None;
                    ai.has_patrol_path = false;
                }

                // -- TransformPatrolIDsToRealPatrol --
                // Runs exactly once at AI init from the enemy AI's
                // `init_ai` before the first `initialize_patrol()`.
                // The raw mission subordinate IDs live on LevelAssets,
                // not on the serialized AI controller; runtime patrol
                // rebuilds use `theoretical_patrol`.
                if let Some(soldier_load_index) =
                    all_soldier_entity_ids.iter().position(|&eid| eid == npc_id)
                    && let Some(patrol_ids) = soldier_subordinate_ids.get(soldier_load_index)
                    && !patrol_ids.is_empty()
                {
                    ai.patrol.clear();
                    ai.missed_patrol_members.clear();
                    ai.theoretical_patrol.clear();
                    for &id in patrol_ids {
                        if let Some(&eid) = all_soldier_entity_ids.get(id as usize) {
                            ai.theoretical_patrol.push(eid);
                        } else {
                            tracing::warn!(
                                "NPC {} patrol ID {} out of range (max {})",
                                npc_id.index(),
                                id,
                                all_soldier_entity_ids.len()
                            );
                        }
                    }
                }
            }
        }

        // -- Phase 7: Dispatch to the subclass for state transitions. --
        // The InitState / ReturnToDuty / beggar-lock tail.  The subclass
        // commits the AI-side state transition via
        // `AiController::init_state` and returns the entity-side side
        // effects — posture / action state / eye status / life-point /
        // concussion writes that the AI layer can't reach on its own.
        let init_fx: crate::ai::InitStateSideEffects = {
            let Some(entity) = self.entities.get_mut(npc_id) else {
                return;
            };
            match &mut entity.npc_data_mut().map(|n| &mut n.ai_brain) {
                Some(crate::element::AiBrain::Enemy(e)) => {
                    // Init runs during `InitOneAI` before any detection
                    // or target selection — `primary_target` is 0 and
                    // no battle context exists yet, so the centralized
                    // `build_npc_tick_data` would return `stub()`
                    // anyway.  Skip the round-trip and pass stub
                    // directly.
                    let tick = AiPerTickData::stub();
                    e.init_one_ai(&init_ctx, &tick)
                }
                Some(crate::element::AiBrain::Friendly(f)) => f.init_one_ai(&init_ctx),
                _ => return,
            }
        };

        // -- Phase 8: Apply entity-side side effects from `init_state`. --
        // Posture, action state, eye status, life points, and
        // concussion all live on the entity, not the AI brain.  The
        // subclass dispatch already committed the AI-side state
        // transitions; here we flush the entity half.
        if init_fx.set_posture.is_some()
            || init_fx.set_action_state.is_some()
            || init_fx.set_eye_status.is_some()
            || init_fx.zero_life_points
            || init_fx.concussion_max_and_unconscious
        {
            let Some(entity) = self.entities.get_mut(npc_id) else {
                return;
            };

            // Posture: write to `ElementData::posture`.  Matches the
            // existing `pending_posture` drain path which deliberately
            // skips `PositionInterface::set_posture` — the move-box
            // recomputation is deferred and every other posture write
            // in the codebase (melee knock-out paths,
            // ability.CarryingCorpse, …) follows the same pattern.
            if let Some(posture) = init_fx.set_posture {
                entity.set_posture(posture);
            }

            // Action state: write on `ActorData`.  `set_states(...,
            // action_state)` + `wait()` collapse to a direct
            // `action_state = X` at init time, since the entity has no
            // active animation to interrupt.
            if let Some(action_state) = init_fx.set_action_state
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.action_state = action_state;
            }

            // Eye status: use the existing `ai_vision::set_view_status`
            // helper so `view_transition` is flipped alongside the raw
            // field.  Equivalent to `close_eyes` (which just calls
            // `set_view_status(EYES_CLOSED)`).
            if let Some(status) = init_fx.set_eye_status
                && let Some(npc) = entity.npc_data_mut()
            {
                crate::ai_vision::set_view_status(npc, status);
            }

            // Zero life points + killed-by-accident.  Bundled because
            // they are always written together at init
            // (`init_with_zero_life_points` followed by
            // `set_killed_by_accident(true)`).
            if init_fx.zero_life_points {
                if let Some(npc) = entity.npc_data_mut() {
                    npc.life_points = 0;
                }
                if let Some(human) = entity.human_data_mut() {
                    human.killed_by_accident = true;
                }
            }

            // Max concussion + unconscious.  Init-time bypasses
            // the full `combat::set_concussion` state machine
            // because none of its gates (script lock, tied,
            // carried) apply on a freshly-spawned NPC.
            if init_fx.concussion_max_and_unconscious
                && let Some(human) = entity.human_data_mut()
            {
                human.concussion_of_the_brain = crate::combat::CONCUSSION_MAX;
                human.unconscious = true;
            }
        }
    }

    /// Populate `AiGlobalState::houses` and `door_rally_points` from
    /// the currently-loaded level, and assign `leave_house_number` to
    /// each NPC occupant.
    ///
    /// The building-loop portion of AI init.  Each building sector
    /// becomes one [`House`]: its doors are looked up via the door
    /// table (doors whose `sector_in` matches the building), its
    /// occupants are found by scanning entities currently in that
    /// sector, and one [`DoorRallyPoint`] is anchored at every door's
    /// `point_out`.
    ///
    /// Runtime occupant tracking is wired at the `execute_pass_door`
    /// Enter / Leave branches in `engine::door_pass`: the same hook
    /// that updates `game_host.building_occupants` also updates
    /// `House::occupant_ids`.
    pub(super) fn initialize_buildings(&mut self) {
        use crate::ai::{AI_DOOR_RALLY_POINT_DISTANCE, DoorRallyPoint, House, Position};

        self.ai_global.houses.clear();
        self.ai_global.door_rally_points.clear();

        // Index doors by their `sector_in` (building interior side).
        // We read from the live door table on the game host.
        // BTreeMap (not HashMap) so the `for (sector_in, …) in
        // doors_by_building` iteration below assigns `leave_house_number`
        // in a stable, sector-ordered sequence — replay/lockstep multi-
        // player need deterministic AI state.
        let mut doors_by_building: std::collections::BTreeMap<
            crate::sector::SectorNumber,
            Vec<u32>,
        > = std::collections::BTreeMap::new();
        let mut rally_points: Vec<DoorRallyPoint> = Vec::new();

        // Include every building's doors — not just those occupied
        // at init time — so the runtime enter/leave hooks have a
        // pre-existing `House` to update when an NPC walks into a
        // previously-empty building.  The reference's restriction to
        // NPC-populated buildings (the houses list being built from
        // starting sectors) is an artifact of *how* it initializes,
        // not a semantic invariant; live occupant tracking supersedes
        // it.  Trap doors (`BuildingTrap`) remain excluded — those
        // sectors aren't regular building interiors and shouldn't
        // carry rally points.
        if let Some(game_host) = self.mission_script.as_ref().and_then(|s| s.game_host()) {
            for (idx, door) in game_host.doors.iter().enumerate() {
                if !matches!(door.door_type, crate::gate::DoorType::Building) {
                    continue;
                }
                doors_by_building
                    .entry(door.sector_in)
                    .or_default()
                    .push(idx as u32);

                // Rally point: use the door's `point_out` directly
                // (the sectorised "outside" position).
                rally_points.push(DoorRallyPoint {
                    position: Position {
                        x: door.point_out.x,
                        y: door.point_out.y,
                        sector: crate::position_interface::SectorHandle::new(u16::from(
                            door.sector_out,
                        )),
                        level: door.layer_out,
                    },
                    door_index: crate::gate::DoorIndex(idx as u32),
                    radius: AI_DOOR_RALLY_POINT_DISTANCE,
                });
            }
        }

        // Collect occupants per building from the current entity set.
        // An entity is "in building X" if its sector is X *and* that
        // sector is flagged `is_building()`.  We skip entities without
        // actor data (objects, FX) since only actors can be NPCs.
        let mut occupants_by_building: std::collections::HashMap<
            crate::sector::SectorNumber,
            Vec<EntityId>,
        > = std::collections::HashMap::new();

        for (entity_id, entity) in self.entities.actors() {
            let elem = entity.element_data();
            let sector_raw = match elem.sector() {
                Some(s) => crate::sector::SectorNumber::new(u16::from(s) as i16),
                None => continue,
            };
            // Only record occupants of sectors we know are buildings
            // (have at least one building door pointing at them).
            if !doors_by_building.contains_key(&sector_raw) {
                continue;
            }
            occupants_by_building
                .entry(sector_raw)
                .or_default()
                .push(entity_id.into());
        }

        // Build the houses list from the collected door/occupant maps.
        for (sector_in, door_indices) in doors_by_building {
            let occupant_ids = occupants_by_building.remove(&sector_in).unwrap_or_default();

            // Distribute sequential `leave_house_number` to each
            // occupant — used by the departure scheduler to stagger
            // NPCs exiting during alerts.
            for (n, &eid) in occupant_ids.iter().enumerate() {
                if let Some(entity) = self.entities.get_mut(eid)
                    && let Some(ai) = entity.ai_controller_mut()
                {
                    ai.leave_house_number = n as u16;
                }
            }

            // Look up the building index on the grid sector.  `None`
            // for script-synthesised or otherwise proto-unlinked
            // building sectors — rare but non-fatal.
            let building_index = self
                .fast_grid
                .level
                .sector_number_map
                .get(&sector_in)
                .and_then(|&idx| self.fast_grid.level.sectors.get(idx))
                .and_then(|gs| gs.building_index);

            // Read `arrow_reserve` off the parallel `GameHost` array
            // (populated from the GUYS/CAVE tenant chunk at level
            // load).  `max_occupants` still has no proto source — we
            // leave it at the `0xFFFF` default (unlimited) matching
            // `BuildingData::default()`.
            let arrow_reserve = building_index
                .and_then(|bi| {
                    self.mission_script
                        .as_ref()
                        .and_then(|s| s.game_host())
                        .and_then(|h| h.arrow_reserves.get(usize::from(bi)).copied())
                })
                .unwrap_or(false);

            self.ai_global.houses.push(House {
                sector_index: u32::from(u16::from(sector_in)),
                building_index,
                door_indices,
                occupant_ids,
                arrow_reserve,
            });
        }

        self.ai_global.door_rally_points = rally_points;

        tracing::info!(
            houses = self.ai_global.houses.len(),
            rally_points = self.ai_global.door_rally_points.len(),
            "Initialized AI building data"
        );
    }

    /// Recompute overall villain alert status from soldier NPCs, updating
    /// global counters and triggering combat/alert music transitions.
    ///
    /// Ports the per-NPC work of `change_alert_status` into a
    /// single-shot sweep that runs once per frame. The per-NPC
    /// `set_alert_status` already writes `current_music_alert_status`
    /// but doesn't touch the global counters or call
    /// `set_music_mode`; this method fills that gap.
    ///
    /// Call once per frame before the sound `hourglass` so a transition
    /// to yellow/red promptly bumps the music pool weight.
    pub(crate) fn update_overall_villain_alert(
        &mut self,
        profiles: &crate::profiles::ProfileManager,
    ) {
        let mut yellow = 0u16;
        let mut red = 0u16;
        let mut green = 0u16;
        // Per-call `ALERT_INSTANT_MUSIC_CHANGE` flag is staged on each
        // AiController by `set_alert_status_with_flags`; OR it across
        // soldiers here and clear after consumption.  Non-soldier flags
        // are ignored to match the soldier-only gate.
        let mut any_instant_change = false;
        for (_, soldier) in self.entities.soldiers_mut() {
            let Some(ai) = soldier.npc.ai_brain.base_mut() else {
                continue;
            };
            match ai.current_music_alert_status {
                crate::ai::AlertLevel::Green => green += 1,
                crate::ai::AlertLevel::Yellow => yellow += 1,
                crate::ai::AlertLevel::Red => red += 1,
            }
            if ai.pending_instant_music_change {
                any_instant_change = true;
                ai.pending_instant_music_change = false;
            }
        }
        self.ai_global.green_alert_soldiers = green;
        self.ai_global.yellow_alert_soldiers = yellow;
        self.ai_global.red_alert_soldiers = red;

        let new_overall = self.ai_global.overall_villain_alert();
        if new_overall == self.ai_global.overall_villain_alert_status {
            return;
        }
        let prev = self.ai_global.overall_villain_alert_status;
        self.ai_global.overall_villain_alert_status = new_overall;
        self.ai_global.overall_alert_status = new_overall;

        // Only call `set_music_mode` when not in Sherwood.  Sherwood
        // has its own ambient track and shouldn't hear combat/alert
        // cues even if a soldier briefly goes yellow.
        let is_sherwood = self
            .campaign
            .as_ref()
            .and_then(|c| c.current_mission_idx)
            .and_then(|idx| self.campaign.as_ref().and_then(|c| c.missions.get(idx)))
            .is_some_and(|m| {
                m.profile(profiles).location == crate::profiles::MissionLocation::Sherwood
            });

        if !is_sherwood {
            use crate::sound::MusicMode;
            // On the Green arm, forest levels keep the alert track
            // instead of dropping to quiet so the woodland ambient
            // layer keeps playing under any residual yellow soldiers.
            let mode = match new_overall {
                crate::ai::AlertLevel::Green => {
                    if self.weather.is_forest_level {
                        MusicMode::Alert
                    } else {
                        MusicMode::Quiet
                    }
                }
                crate::ai::AlertLevel::Yellow => MusicMode::Alert,
                crate::ai::AlertLevel::Red => MusicMode::Fight,
            };
            // `set_alert_status` calls `force_music_mode` when the
            // caller passes `ALERT_INSTANT_MUSIC_CHANGE`.  Known
            // shipped call sites are all Green-target (two AI sites
            // and the NPC death path).  The flag is now staged per-NPC
            // on `AiController::pending_instant_music_change` by
            // `set_alert_status_with_flags`; the sweep above OR'd it
            // across soldiers into `any_instant_change`, so any
            // transition direction passing the flag forces immediately.
            let cmd = if any_instant_change {
                super::SoundCommand::ForceMusicMode(mode)
            } else {
                super::SoundCommand::SetMusicMode(mode)
            };
            self.pending_side_effects.sounds.push(cmd);
        }

        tracing::debug!(
            "Overall villain alert {:?} → {:?} (green={green} yellow={yellow} red={red})",
            prev,
            new_overall,
        );
    }

    /// Set every NPC's `view_radius_base`, `view_radius_goal`, and
    /// `view_radius` from `standard_view_polygon_radius`.  Called at
    /// init and when the script changes the radius at runtime.
    pub(super) fn propagate_view_radius(&mut self) {
        let r = if self.standard_view_polygon_radius > 0 {
            self.standard_view_polygon_radius
        } else {
            ai_vision::DEFAULT_VIEW_RADIUS
        };
        for (_, entity) in self.entities.npcs_mut() {
            if let Some(npc) = entity.npc_data_mut() {
                npc.view_radius_base = r;
                npc.view_radius_goal = r;
                npc.view_radius = r;
            }
        }
    }

    // ─── Turn order processing ──────────────────────────────────

    /// Process pending turn orders from NPC order queues.
    ///
    /// `face_direction` / `face_position` produce `Turning` orders that
    /// `process_pending_ai_orders` routes to `actor.order_queue`.
    /// These become `Turn` sequence elements that complete in one
    /// frame and fire `EventDone`.  We replicate that here: set the
    /// entity's direction toward the target position, then dispatch
    /// `EventDone` so the AI state machine continues.
    /// Drain animation-type orders (Pointing, RaisingShield, LoweringShield,
    /// Menacing, etc.) from NPC order queues and start them as `active_ai_anim`.
    /// Like `process_turn_orders` but for multi-frame animations that
    /// need EventDone when the sprite animation completes.
    pub(super) fn process_animation_orders(&mut self) {
        // Legacy entry point — left as a no-op now that the animation
        // driver reads the front order directly via
        // `current_order_for_actor`.  Animations booked onto sequence
        // elements are picked up automatically; there is no longer a
        // separate drain-and-rebook step.
    }

    pub(super) fn process_turn_orders(&mut self) {
        use crate::order::OrderType;
        use crate::position_interface::vector_to_sector_0_to_15_iso;

        // `TurnFast` sets the direction goal on the
        // `PositionInterface` and drops a single `Turning` order onto
        // the sequence element.  The animation driver special-cases
        // `Turning` and drives `turn_fast()` each tick until the body
        // reaches the goal; the default `advance_element` completion
        // then pops the order and terminates the element.
        //
        // All that remains here is to apply the goal direction on the
        // actor — the order itself is already sitting on the Turn
        // element (pushed by `Command::Turn` in tick.rs or by
        // `process_pending_ai_orders` for free-standing turn commands).
        //
        // `Turn` computes the goal via
        // `vector_to_sector_0_to_15_iso` — i.e. applies the isometric
        // aspect correction to dy.  `face_position_impl` (which
        // produces the target position stored in the order) also
        // applies the correction.  We MUST match it here; otherwise
        // the actor gets snapped to a sector 1 off from what the AI
        // computed, which breaks `FaceTo` early-return parity and
        // causes a spurious 18-frame Turn animation blocking the
        // REACTIONTIME_TURNING → REACTIONTIME transition (~5 visible
        // frames of "standing alerted" before the enemy starts
        // running).
        let mut updates: Vec<(crate::element::EntityId, i16)> = Vec::new();
        for (npc_id, entity) in self.entities.npcs() {
            let Some((_, _, front)) = self.sequence_manager.current_order_for_actor(npc_id) else {
                continue;
            };
            if front.order_type != OrderType::Turning {
                continue;
            }
            let pos = entity.element_data().position_map();
            let dx = front.target_x - pos.x;
            let dy = front.target_y - pos.y;
            let new_dir = vector_to_sector_0_to_15_iso(dx, dy);
            updates.push((npc_id.into(), new_dir));
        }

        for (npc_id, new_dir) in updates {
            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };
            if entity.actor_data().is_some() {
                entity.position_iface_mut().set_direction(
                    crate::position_interface::Direction::from_raw(new_dir as i32),
                );
            }
        }
    }

    // ─── EventReachPoint dispatch ───────────────────────────────

    /// Dispatch `EventReachPoint` stimulus to NPCs whose movement just
    /// completed.
    ///
    /// `send_condolation_card` calls `think(EVENT_REACHPOINT)` when a
    /// MOVE sequence element reaches the terminated state.  Originally
    /// this fired from inside the sequence manager's state-change
    /// callback; here we collect the arrivals from
    /// `tick_entity_movement` and dispatch in this pass.
    ///
    /// Any new orders produced by the AI (e.g. "walk to next waypoint")
    /// will be drained on the next frame by `process_pending_ai_orders`.
    pub(super) fn dispatch_reach_point_events(
        &mut self,
        assets: &LevelAssets,
        entities: &[EntityId],
    ) {
        let scratch = self.build_sim_scratch(assets);
        let current_frame = self.frame_counter;

        for &entity_id in entities {
            // Build ctx in a read-only scope so we can then call
            // `dispatch_filtered_stimulus`, which needs `&mut self`.
            let in_uninterruptible_command = self.is_very_very_busy(entity_id);
            let ctx = {
                let Some(entity) = self.entities.get(entity_id) else {
                    continue;
                };
                let mut ctx = build_ai_context_from_entity(
                    entity,
                    current_frame,
                    None,
                    self.weather.is_forest_level,
                    self.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.fast_grid,
                    &assets.hiking_paths,
                    &self.ai_global.all_soldier_handles,
                );
                ctx.in_uninterruptible_command = in_uninterruptible_command;
                ctx
            };
            let stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventReachPoint);
            // Centralized builder: assembles primary target metadata,
            // friend-swap candidates, avenger-on-roof wait position,
            // and a seeded enemy_sq_distances.  Non-enemy-NPC entities
            // get a stub.
            let tick_data = self.build_npc_tick_data(entity_id, &scratch, assets);

            self.dispatch_think_with_drain(entity_id, &stimulus, &ctx, &tick_data, assets);
        }
    }

    // ─── EventGaloppLoopEnd dispatch ────────────────────────────

    /// Dispatch `EventGaloppLoopEnd` to riders with `RHMOVE_RIDER_CHARGE`
    /// flag that reached an intermediate waypoint during movement.
    ///
    /// When a rider's running animation reaches half/end frame with
    /// the `RIDER_CHARGE` move flag, `think(EVENT_GALOPP_LOOP_END)`
    /// fires so the AI can call `maybe_make_rider_attack()` to check
    /// if it's close enough to begin the actual charge pass.
    pub(super) fn dispatch_galopp_loop_events(
        &mut self,
        assets: &LevelAssets,
        entities: &[EntityId],
    ) {
        let scratch = self.build_sim_scratch(assets);
        let current_frame = self.frame_counter;

        for &entity_id in entities {
            let ctx = {
                let Some(entity) = self.entities.get(entity_id) else {
                    continue;
                };
                build_ai_context_from_entity(
                    entity,
                    current_frame,
                    None,
                    self.weather.is_forest_level,
                    self.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.fast_grid,
                    &assets.hiking_paths,
                    &self.ai_global.all_soldier_handles,
                )
            };

            let stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventGaloppLoopEnd);
            // EventGaloppLoopEnd fires on enemy riders mid-charge
            // towards their primary target — the AI inspects the
            // target position to decide whether to begin the attack
            // pass.  Populate primary-target metadata via the builder.
            let tick_data = self.build_npc_tick_data(entity_id, &scratch, assets);

            self.dispatch_think_with_drain(entity_id, &stimulus, &ctx, &tick_data, assets);
        }
    }

    /// Per-frame enemy-AI perception tick.
    ///
    /// The `refresh_detection` loop, specialised to `DETECTABLE_ENEMY`
    /// for Lacklandist soldiers hunting PCs.  See
    /// `ai_vision::compute_visibility` for the perception primitive.
    ///
    /// High-level flow:
    ///
    ///  1. Build a snapshot of all alive / playable PCs.
    ///  2. For each alive non-locked hostile soldier:
    ///     a. Compute the per-NPC `uwModifiedFrameCounter` phase.
    ///     b. If the `DETECTION_FREQUENCY_ENEMY_PC` gate is open,
    ///     call `compute_visibility` against each PC and multiply
    ///     by `DETECTION_FREQUENCY_ENEMY_PC`.
    ///     c. Turn the visibility into `sharpness = BASE_VIEW_SPEED
    ///        * visibility`.
    ///     d. Accumulate sharpness into the NPC's
    ///        `detection_suspects[ENEMY]`, respecting the
    ///        "only add new sightings" edge trigger.
    ///     e. Commit a detection when either `suspects >= 1000` OR
    ///        `instant_detection(type) && sum > 0`.
    ///     f. If nothing is visible this frame, decay suspects on a
    ///        `UNSUSPECT_FREQUENCY` cadence.
    ///  3. On commit: flip the NPC's `AiState` to `Attacking`, store
    ///     the target on `ai_controller.primary_target`, mark the
    ///     NPC `alerted`, and dispatch a pursuit path.
    ///
    /// # What is deferred
    ///
    /// These pieces are either stubbed or skipped because they need
    /// subsystems that aren't ported yet; each is noted inline at the
    /// point it would slot back in.
    ///
    ///  * Full stimulus → `think(STIMULUS_SEE_ENEMY)` dispatch.  The
    ///    reference emits stimuli into the state machine and lets `think()`
    ///    handle reaction time, officer escalation, pre-detection
    ///    animations, etc.  We set `current_state = Attacking`
    ///    directly because the `EnemyAi` wrapper isn't attached to
    ///    the entity yet (only the base `AiController` is).
    ///  * `SIGHTOBSTACLE_OPAQUE` LOS — see
    ///    `ai_vision::los_clear` (uses motion lines as a proxy until
    ///    `SightObstacle` is wired into `FastFindGrid`).
    ///  * View-parameter eye-state check, `IsBuilding()` sector-side
    ///    checks, and the forest merry-men 180° special case — see
    ///    the notes inside `compute_visibility`.
    ///  * `SelectPrimaryTarget` priority scoring — we take the first
    ///    visible PC with the highest sharpness this frame.
    ///  * The `Attacking` substate machine for pursuit — we ask the
    ///    pathfinder to chase the live PC position every
    ///    `PURSUIT_REPATH_INTERVAL` frames.
    ///  * Lost-sight → `Seeking` fallback.  Once alerted, the NPC
    ///    stays alerted (`npc.alerted = true`) until the entity is
    ///    removed; ideally we'd transition back to Default after
    ///    losing the trail.
    ///
    /// Map a PC's currently-executing animation (`OrderType`) to the
    /// noise volume they produce, via a per-animation switch in
    /// `refresh_produced_noise`.
    ///
    /// `refresh_produced_noise` runs from `hourglass()` each frame and
    /// reads `get_animation()` — the currently-running animation — to
    /// set `currently_produced_noise.volume`.  We reproduce the same
    /// lookup here from the PC's active `OrderType` (peeked from
    /// `actor.order_queue`, the equivalent of the sequence slot that
    /// `get_animation` reads).
    ///
    /// Material selects the walk/run/drop volume (wood = loud, grass =
    /// quiet, water = noisiest, light-shadow = silent).  The jump,
    /// sword-fight and breath volumes are material-independent.
    ///
    /// Returns `0` when the PC is inside a building or inactive, or
    /// when the animation doesn't map to any of the noise cases —
    /// matching the `inside_building || !active` early-out.
    fn pc_noise_volume(
        order_type: crate::order::OrderType,
        material: crate::element::GameMaterial,
        in_building: bool,
        active: bool,
        prev_volume: u16,
    ) -> u16 {
        use crate::element::GameMaterial as Material;
        use crate::order::OrderType as OT;

        // When the actor is inside a building or inactive, the volume
        // is forced to 0.  Hearing then becomes impossible because the
        // hear-my-noise box collapses.
        if in_building || !active {
            return 0;
        }

        // Walk/run/drop volumes per material.
        let (walk, run, drop) = match material {
            Material::Ground => (20, 70, 50),
            Material::Wood => (40, 150, 100),
            Material::Stone => (20, 75, 50),
            Material::Grass => (40, 150, 100), // GRASS_DRY
            Material::Leaves => (10, 50, 30),  // GRASS_FRESH
            Material::Water => (200, 400, 300),
            Material::Bush => (40, 150, 100),
            Material::Ice => (20, 75, 50),
            // LightShadow has no assignment in either the walk or run
            // switch, so `volume` keeps whatever it was on the prior
            // frame.  Substitute `prev_volume` for the walk and run
            // slots.  `drop` (Rolling / CarryingCorpse) has no
            // counterpart in `refresh_produced_noise`, so keep the
            // pre-existing 50 fallback.
            Material::LightShadow => (prev_volume, prev_volume, 50),
            _ => (20, 70, 50), // default = ground
        };

        // NOISE_VOLUME_* constants.
        const BREATH: u16 = 15;
        const SWORDFIGHT: u16 = 200;
        const JUMP_UP: u16 = 50;
        const JUMP_LONG: u16 = 50;
        const JUMP_DOWN: u16 = 80;

        match order_type {
            // ── BREATH: idle, bow aim, sitting, freezing ──
            OT::WaitingUprightBored
            | OT::WaitingUprightBoredRandom
            | OT::WaitingUpright
            | OT::WaitingCrouched
            | OT::TransitionEquipBow
            | OT::TransitionUnequipBow
            | OT::TransitionLoadingBow
            | OT::TransitionUnloadBow
            | OT::TransitionRaisingBow
            | OT::TransitionLoweringBow
            | OT::AimingWithBow
            | OT::AimingWithBowUp
            | OT::ShootingWithBow
            | OT::ShootingWithBowUp
            | OT::Freezing
            | OT::WaitingFreeLift
            | OT::Sitting
            | OT::TransitionWaitingUprightSitting
            | OT::TransitionSittingWaitingUpright => BREATH,

            // ── WALK (material-dependent) ──
            OT::WalkingUpright
            | OT::TransitionWaitingUprightBoredWaitingUpright
            | OT::TransitionWaitingUprightWaitingUprightBored
            | OT::TransitionWaitingUprightWalkingUpright
            | OT::WalkingStairs
            | OT::TransitionCrouchingUp
            | OT::TransitionCrouchingDown
            | OT::TransitionWaitingUprightClimbingWallUp
            | OT::ClimbingWallUp
            | OT::ClimbingWallDown
            | OT::TransitionClimbingWallUpWaitingCrouchedCrenel
            | OT::TransitionWaitingCrouchedClimbingWallDownCrenel
            | OT::TransitionClimbingWallUpWaitingCrouched
            | OT::TransitionClimbingWallDownWaitingUpright
            | OT::TransitionWaitingCrouchedClimbingWallDown
            | OT::TransitionWaitingUprightClimbingLadderUp
            | OT::ClimbingLadderUp
            | OT::TransitionClimbingLadderUpWaitingCrouched
            | OT::TransitionWaitingCrouchedClimbingLadderDown
            | OT::ClimbingLadderDown
            | OT::TransitionClimbingLadderDownWaitingUpright
            | OT::StandingUp
            | OT::Turning
            | OT::TransitionWalkingUprightWaitingUpright
            | OT::PassingDoor
            | OT::WalkingWithSword
            | OT::TransitionWaitingCrouchedWalkingCrouched
            | OT::WalkingCrouched
            | OT::TransitionWalkingCrouchedWaitingCrouched
            | OT::TransitionWalkingUprightWalkingCrouched
            | OT::TransitionWalkingCrouchedWalkingUpright => walk,

            // ── RUN (material-dependent) ──
            OT::RunningUpright
            | OT::TransitionWalkingUprightRunningUpright
            | OT::TransitionRunningUprightWalkingUpright
            | OT::TransitionRunningUprightWaitingUpright
            | OT::TransitionWaitingUprightRunningUpright
            | OT::TransitionRunningUprightWalkingCrouched
            | OT::TransitionWalkingCrouchedRunningUpright
            | OT::RunningStairs
            | OT::ClimbingLadderUpFast
            | OT::ClimbingLadderDownFast
            | OT::RunningWithSword => run,

            // ── JUMP land transitions ──
            OT::TransitionJumpingUpWaitingCrouched => JUMP_UP,
            OT::TransitionJumpingLongWaitingUpright
            | OT::TransitionJumpingLongSwordWaitingSword => JUMP_LONG,
            OT::TransitionJumpingDownWaitingCrouched => JUMP_DOWN,

            // ── SWORDFIGHT ──
            OT::StrikingRightSmalltalk
            | OT::StrikingLeftSmalltalk
            | OT::ParryingRightSmalltalk
            | OT::ParryingLeftSmalltalk
            | OT::StrikingLowRightSmalltalk
            | OT::StrikingLowLeftSmalltalk
            | OT::ParryingLowRightSmalltalk
            | OT::ParryingLowLeftSmalltalk
            | OT::StrikingStraightSword
            | OT::StrikingStraightStrongSword
            | OT::StrikingRightSword
            | OT::StrikingLeftSword
            | OT::StrikingRoundRightSword
            | OT::StrikingRoundLeftSword
            | OT::StrikingSemiroundRightSword
            | OT::StrikingSemiroundLeftSword
            | OT::ExecutingSword
            | OT::TransitionWaitingSwordParryingSword
            | OT::ParryingSword
            | OT::TransitionParryingSwordWaitingSword
            | OT::ParryingLowSword
            | OT::Provoking
            | OT::StrikingDownSword => SWORDFIGHT,

            // ── DROP (material-dependent) ──
            OT::Rolling | OT::TransitionCarryingCorpseWaitingUpright => drop,

            // Everything else (injuries, death, bow injuries, menacing,
            // beggar, climbing shoulders, drinking, etc.) — silent.
            _ => 0,
        }
    }

    /// Test whether the entity is inside a building: the
    /// building-sector flag OR the door-transit branch — true during
    /// the few frames an actor is on a door whose inside-sector is a
    /// building but whose current sector pointer has not yet been
    /// swapped.
    pub(super) fn entity_data_inside_building(&self, elem: &crate::element::ElementData) -> bool {
        self.entity_building_sector(elem.sector()).is_some() || elem.is_in_door_transit()
    }

    /// Per-frame sweep that honours `inform_my_friends` on downed NPCs.
    ///
    /// When the flag is set (and the engine isn't frozen) clear it
    /// and call `my_dear_friends_please_please_detect_me` to broadcast
    /// DETECTABLE_BODY to every other NPC.
    pub(super) fn tick_inform_my_friends(&mut self) {
        if self.actors_frozen() {
            return;
        }

        let mut to_broadcast: Vec<EntityId> = Vec::new();
        for (id, entity) in self.entities.npcs_mut() {
            let Some(npc) = entity.npc_data_mut() else {
                continue;
            };
            if npc.inform_my_friends {
                npc.inform_my_friends = false;
                to_broadcast.push(id.into());
            }
        }

        for body_id in to_broadcast {
            self.broadcast_body_detectable(body_id);
        }
    }

    /// Iterates every NPC except the body itself and registers the
    /// body under DETECTABLE_BODY.
    #[tracing::instrument(level = "trace", skip_all, fields(body = body_id.index()))]
    pub(super) fn broadcast_body_detectable(&mut self, body_id: EntityId) {
        use crate::element::DetectableType;

        // Snapshot the body's position + `knocked_out_in_money_fight`
        // flag for the per-friend radius check below.
        let (body_pos, body_knocked_out_in_money_fight, body_is_soldier) = {
            let Some(entity) = self.entities.get_mut(body_id) else {
                return;
            };
            let is_soldier = matches!(entity, Entity::Soldier(_));
            let pos = entity.element_data().position_map();
            let ko = entity
                .npc_data()
                .and_then(|n| n.ai_brain.base())
                .map(|b| b.knocked_out_in_money_fight)
                .unwrap_or(false);
            (pos, ko, is_soldier)
        };

        // Append to every other NPC's Body detectable list (skip duplicates).
        // The NPC list holds both soldiers and civilians, so civilian
        // NPCs must receive the body detectable too — otherwise
        // `get_worst_detected_type` never climbs past DETECTABLE_FRIEND
        // for civilians, dropping their emoticon / alert reactions to
        // nearby bodies.
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        let det_idx = DetectableType::Body as usize;
        for friend_id in npc_ids {
            if friend_id == body_id {
                continue;
            }
            let Some(entity) = self.entities.get_mut(friend_id) else {
                continue;
            };
            let friend_pos = entity.element_data().position_map();
            let Some(npc) = entity.npc_data_mut() else {
                continue;
            };

            // If this body was knocked out during a money fight, only
            // register the body with friends beyond
            // `AI_DOLLAR_FIGHT_IGNORE_BODY_RADIUS` (Chebyshev
            // distance).  Close-by money-fight participants
            // deliberately ignore the downed fighter.
            let add_detectable = if body_knocked_out_in_money_fight {
                let dx = (body_pos.x - friend_pos.x).abs();
                let dy = (body_pos.y - friend_pos.y).abs();
                dx.max(dy) > crate::parameters_ai::AI_DOLLAR_FIGHT_IGNORE_BODY_RADIUS as f32
            } else {
                true
            };

            if add_detectable && det_idx < npc.detectable_lists.len() {
                let already = npc.detectable_lists[det_idx]
                    .iter()
                    .any(|d| d.element == Some(body_id));
                if !already {
                    npc.detectable_lists[det_idx].push(crate::element::Detectable {
                        element: Some(body_id),
                        detectable_type: DetectableType::Body,
                        ..Default::default()
                    });
                }
            }

            // Also remove the body from the friend's
            // money-fight-enemies list when both are soldiers.  Runs
            // unconditionally of the radius check.  Civilians have no
            // `EnemyAi`, so `enemy_mut()` is None and this arm is a
            // natural no-op for them — only soldiers track money-fight
            // enemies.
            if body_is_soldier && let Some(enemy_ai) = npc.ai_brain.enemy_mut() {
                enemy_ai
                    .money_fight_enemies
                    .retain(|h| *h != body_id.index());
            }
        }
    }

    /// Remove `beggar_id` from every NPC's `DETECTABLE_BEGGAR` list.
    /// Once any seek-area soldier has claimed the PC-beggar (queued it into
    /// `beggars_to_control`), this sweeps the beggar out of every
    /// soldier's and civilian's BEGGAR list so no other soldier fires
    /// a duplicate `EVENT_SEES_BEGGAR` on subsequent frames.
    ///
    /// Modelled on `engine/nets.rs:delete_body_detectable_for_all_npc`
    /// but hardcoded to `DetectableType::Beggar`.
    #[tracing::instrument(level = "trace", skip_all, fields(beggar = beggar_id.index()))]
    pub(super) fn delete_beggar_detectable_for_all_npc(&mut self, beggar_id: EntityId) {
        use crate::element::DetectableType;
        let det_idx = DetectableType::Beggar as usize;
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for friend_id in npc_ids {
            let Some(entity) = self.entities.get_mut(friend_id) else {
                continue;
            };
            let Some(npc) = entity.npc_data_mut() else {
                continue;
            };
            if det_idx < npc.detectable_lists.len() {
                npc.detectable_lists[det_idx].retain(|d| d.element != Some(beggar_id));
            }
        }
    }

    /// Per-frame sweep that honours `pending_inform_resurrection` and
    /// `pending_set_eye_status` on any NPC that just regained
    /// consciousness.  Runs `inform_everyone_on_my_resurrection` and
    /// the companion `set_view_status(EYES_LOOK_FORWARD)` — both fired
    /// from the civilian `EVENT_FITAGAIN` handler.
    ///
    /// Runs after `tick_inform_my_friends` so a "down → up → down"
    /// flicker in the same frame resolves to the freshest state.
    pub(super) fn tick_ai_pending_resurrection_and_eyes(&mut self) {
        if self.actors_frozen() {
            return;
        }

        let mut to_broadcast: Vec<EntityId> = Vec::new();
        let mut to_set_eye: Vec<(EntityId, crate::element::EyeStatus)> = Vec::new();
        for (id, entity) in self.entities.npcs_mut() {
            let Some(ai) = entity.ai_controller_mut() else {
                continue;
            };
            if ai.pending_inform_resurrection {
                ai.pending_inform_resurrection = false;
                to_broadcast.push(id.into());
            }
            if let Some(status) = ai.pending_set_eye_status.take() {
                to_set_eye.push((id.into(), status));
            }
        }

        for resurrected_id in to_broadcast {
            self.broadcast_resurrection(resurrected_id);
        }

        for (npc_id, status) in to_set_eye {
            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };
            if let Some(npc) = entity.npc_data_mut() {
                crate::ai_vision::set_view_status(npc, status);
            }
        }
    }

    /// Remove `resurrected_id` from every other NPC's
    /// `DETECTABLE_BODY` list.  The per-NPC body of
    /// `inform_on_resurrection` — the engine-side fan-out triggered by
    /// `inform_everyone_on_my_resurrection`.
    #[tracing::instrument(level = "trace", skip_all, fields(resurrected = resurrected_id.index()))]
    pub(super) fn broadcast_resurrection(&mut self, resurrected_id: EntityId) {
        use crate::element::DetectableType;
        let det_idx = DetectableType::Body as usize;
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for friend_id in npc_ids {
            if friend_id == resurrected_id {
                continue;
            }
            let Some(entity) = self.entities.get_mut(friend_id) else {
                continue;
            };
            let Some(npc) = entity.npc_data_mut() else {
                continue;
            };
            if det_idx < npc.detectable_lists.len() {
                npc.detectable_lists[det_idx].retain(|d| d.element != Some(resurrected_id));
            }
        }
    }

    /// Per-frame view parameter refresh for every NPC.  The
    /// `refresh_view()` call inside `perform_refresh`.
    pub(super) fn refresh_npc_views(&mut self) {
        if self.actors_frozen() {
            return;
        }

        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for npc_id in npc_ids {
            // ── Phase 1: read-only — gather context ──
            let (ctx, ai_primary_target, ai_last_synced_focus) = {
                let Some(entity) = self.entities.get(npc_id) else {
                    continue;
                };
                let Some(npc) = entity.npc_data() else {
                    continue;
                };

                let edata = entity.element_data();
                let pos = crate::coordinates::MapPoint::new(
                    edata.position_map().x,
                    edata.position_map().y,
                );

                let is_active_and_outside_building =
                    edata.active && !self.entity_data_inside_building(edata);

                let animation = self
                    .sequence_manager
                    .current_order_for_actor(npc_id)
                    .map(|(_, _, o)| o.order_type);

                let is_unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);

                let follow_target_position = npc.follow_target.and_then(|tid| {
                    self.entities.get(tid).map(|e| {
                        let p = &e.element_data().position_map();
                        crate::coordinates::MapPoint::new(p.x, p.y)
                    })
                });

                // Read enemy AI's primary_target, last-synced focus
                // marker, and drunkenness for the focus edge check
                // and wobble inputs.
                let (primary_target, last_synced, blood_alcohol) = entity
                    .enemy_ai()
                    .map(|e| {
                        (
                            e.base.primary_target,
                            e.base.last_synced_focus_target,
                            e.base.blood_alcohol,
                        )
                    })
                    .unwrap_or((0, None, 0));

                (
                    ai_vision::RefreshViewContext {
                        body_direction: edata.direction(),
                        posture: edata.posture,
                        animation,
                        is_unconscious,
                        is_tied: edata.posture == crate::element::Posture::Tied,
                        is_dead: entity.is_dead(),
                        is_active_and_outside_building,
                        is_rider: matches!(entity, Entity::Soldier(s) if s.soldier.rider),
                        blood_alcohol,
                        own_position: pos,
                        follow_target_position,
                    },
                    primary_target,
                    last_synced,
                )
            };
            // shared borrow dropped ──

            // ── Phase 2: mutable — apply refresh_view + focus sync ──
            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };
            // Edge-triggered focus sync: only react when
            // `primary_target` *changed* since the last reconcile.
            // The explicit `pending_focus`/`pending_unfocus` channels
            // are honoured by the drain — they update
            // `last_synced_focus_target` so the next pass sees no
            // edge and won't re-assert focus.  `focus(NULL)` on self
            // is a separate concern from `primary_target` lifecycle
            // (e.g. rider charge passing).
            let ai_primary_target_focus = (ai_primary_target != 0).then_some(ai_primary_target);
            if ai_primary_target_focus != ai_last_synced_focus
                && let Some(npc) = entity.npc_data_mut()
            {
                if let Some(target_handle) = ai_primary_target_focus {
                    ai_vision::focus_entity(
                        npc,
                        EntityId::Pc(crate::entity_id::PcId(target_handle)),
                    );
                } else {
                    ai_vision::unfocus(npc);
                }
            }
            if ai_primary_target_focus != ai_last_synced_focus
                && let Some(ai) = entity.enemy_ai_mut()
            {
                ai.base.last_synced_focus_target = ai_primary_target_focus;
            }
            if let Some(npc) = entity.npc_data_mut() {
                ai_vision::refresh_view(npc, &ctx);
            }
        }
    }

    // ─── NPC speech processing ─────────────────────────────────

    /// Drain pending AI remarks and issue `play_exclamation` calls.
    ///
    /// `Say()` is called inline from the AI state machine and
    /// immediately dispatches to the sound manager.  Here, the AI
    /// stores `current_remark` / `current_remark_flags` during its
    /// tick, and we drain them in this pass — after all AI ticks have
    /// run — to avoid holding a mutable borrow on the entity while
    /// also needing `&mut SoundManager`.
    pub(super) fn process_npc_speech(&mut self, assets: &LevelAssets) {
        use crate::ai::{Remark, RemarkTargetFlags, SpeechFlags};
        use crate::sound::ExclamationGroup;

        let current_frame = self.frame_counter;

        // ── Phase 0: evict expired forbidden remarks ────────────
        self.ai_global
            .forbidden_remarks
            .retain(|fr| fr.forbidden_till_frame >= current_frame);

        // ── Phase 1: snapshot NPC speech data ───────────────────
        // We collect everything needed into a vec to avoid holding
        // mutable borrows on entities while accessing ai_global,
        // profile_manager, and the sound manager.
        struct SpeechSnap {
            entity_id: EntityId,
            remark: Remark,
            flags: u16,
            is_soldier: bool,
            is_vip: bool,
            blipped: bool,
            sector: Option<crate::position_interface::SectorHandle>,
            in_door_transit: bool,
            position: crate::coordinates::MapPoint,
            speech_id: u32,
            script_forbidden: bool,
            profile_name: String,
        }
        let mut snaps: Vec<SpeechSnap> = Vec::new();

        for (npc_id, entity) in self.entities.npcs_mut() {
            let (npc, is_soldier, is_vip, blipped, sector, in_door_transit, pos) = match entity {
                crate::element::Entity::Soldier(s) => (
                    &mut s.npc,
                    true,
                    assets
                        .profile_manager
                        .get_soldier(s.soldier.soldier_profile_index)
                        .map(|p| p.vip)
                        .unwrap_or(false),
                    s.element.blipped,
                    s.element.sector(),
                    s.element.is_in_door_transit(),
                    s.element.position_map(),
                ),
                crate::element::Entity::Civilian(c) => (
                    &mut c.npc,
                    false,
                    assets
                        .profile_manager
                        .civilians
                        .get(usize::from(c.civilian.civilian_profile_index))
                        .map(|p| p.civilian_type == crate::profiles::CivilianType::Vip)
                        .unwrap_or(false),
                    c.element.blipped,
                    c.element.sector(),
                    c.element.is_in_door_transit(),
                    c.element.position_map(),
                ),
                _ => continue,
            };

            let ai = match npc.ai_brain.base_mut() {
                Some(ai) => ai,
                None => continue,
            };

            let remark = ai.current_remark;
            if remark == Remark::TheSoundOfSilence {
                continue;
            }
            // Skip NPCs whose exclamation is already being played by
            // the sound manager — we must NOT redispatch it, and we
            // must NOT clear current_remark (the `already speaking?`
            // guard in `say_impl` reads it to block overrides).
            if ai.speech_in_flight {
                continue;
            }
            let flags = ai.current_remark_flags;

            // Script-forbidden check.
            let script_forbidden = ai.forbidden_remark_ids.contains(&(remark as u32));

            // Get the NPC's speech profile ID and name.
            let (speech_id, profile_name) = match entity {
                crate::element::Entity::Soldier(s) => {
                    let p = assets
                        .profile_manager
                        .get_soldier(s.soldier.soldier_profile_index);
                    (
                        p.map(|p| p.exclamation_id).unwrap_or(0),
                        p.map(|p| p.profile_name.clone()).unwrap_or_default(),
                    )
                }
                crate::element::Entity::Civilian(c) => {
                    let p = assets
                        .profile_manager
                        .civilians
                        .get(usize::from(c.civilian.civilian_profile_index));
                    (
                        p.map(|p| p.exclamation_id).unwrap_or(0),
                        p.map(|p| p.profile_name.clone()).unwrap_or_default(),
                    )
                }
                _ => (0, String::new()),
            };

            tracing::trace!(
                npc = npc_id.index(),
                ?remark,
                flags,
                speech_id,
                blipped,
                script_forbidden,
                profile_name = profile_name.as_str(),
                "process_npc_speech: snap"
            );
            snaps.push(SpeechSnap {
                entity_id: npc_id.into(),
                remark,
                flags,
                is_soldier,
                is_vip,
                blipped,
                sector,
                in_door_transit,
                position: pos,
                speech_id,
                script_forbidden,
                profile_name,
            });
        }

        // ── Phase 2: filter and dispatch ────────────────────────
        // Blocked remarks need MYTALK fired immediately; accepted remarks
        // store flags for Phase 3 callback when sound finishes.
        let mut blocked_mytalk: Vec<(EntityId, u16)> = Vec::new(); // (entity_id, flags)
        let mut accepted_mytalk: Vec<(EntityId, u16)> = Vec::new(); // (entity_id, flags)

        for snap in snaps {
            let flags = SpeechFlags::from_bits_truncate(snap.flags);

            // Blipped → forget it.
            if snap.blipped {
                tracing::trace!(npc = snap.entity_id.index(), ?snap.remark, "speech blocked: blipped");
                blocked_mytalk.push((snap.entity_id, snap.flags));
                continue;
            }

            // Script-forbidden.
            if snap.script_forbidden {
                tracing::trace!(npc = snap.entity_id.index(), ?snap.remark, "speech blocked: script_forbidden");
                blocked_mytalk.push((snap.entity_id, snap.flags));
                continue;
            }

            // Recently-said check (unless SPEECH_ALWAYS).
            // `is_remark_forbidden` with full scope checking.
            if !flags.contains(SpeechFlags::ALWAYS) {
                let is_forbidden = self.ai_global.forbidden_remarks.iter().any(|fr| {
                    if fr.remark != snap.remark {
                        return false;
                    }
                    let scope = RemarkTargetFlags::from_bits_truncate(fr.flags);
                    // THIS_TYPE: same NPC category (soldier/civilian) with same speech profile
                    if scope.contains(RemarkTargetFlags::THIS_TYPE)
                        && fr.bad_guy == snap.is_soldier
                        && fr.speech_id == snap.speech_id
                    {
                        return true;
                    }
                    // THIS_GUY: exact same NPC.  EntityId (monotonic
                    // slot index, never reused) is isomorphic to the
                    // creation counter, so we use it directly in
                    // place of `creation_order`.
                    if scope.contains(RemarkTargetFlags::THIS_GUY)
                        && fr.guy_index == snap.entity_id.index() as u16
                    {
                        return true;
                    }
                    // VILLAINS: all soldiers
                    if scope.contains(RemarkTargetFlags::VILLAINS) && snap.is_soldier {
                        return true;
                    }
                    // CIVILIANS: all civilians
                    if scope.contains(RemarkTargetFlags::CIVILIANS) && !snap.is_soldier {
                        return true;
                    }
                    false
                });
                if is_forbidden {
                    tracing::trace!(npc = snap.entity_id.index(), ?snap.remark, "speech blocked: forbidden_remarks list");
                    blocked_mytalk.push((snap.entity_id, snap.flags));
                    continue;
                }
            }

            // Inside-building suppression (unless SPEECH_HOUSE).
            // `is_inside_building()` — sector-flag OR door-transit.
            if !flags.contains(SpeechFlags::HOUSE) {
                let inside_building =
                    self.entity_building_sector(snap.sector).is_some() || snap.in_door_transit;
                if inside_building {
                    tracing::trace!(npc = snap.entity_id.index(), ?snap.remark, "speech blocked: inside_building");
                    blocked_mytalk.push((snap.entity_id, snap.flags));
                    continue;
                }
            }

            if snap.speech_id == 0 {
                tracing::trace!(npc = snap.entity_id.index(), ?snap.remark, "speech blocked: speech_id=0");
                blocked_mytalk.push((snap.entity_id, snap.flags));
                continue;
            }

            // Resolve remark → (group, exclamation_id).
            let remark_u32 = snap.remark as u32;
            let first_vip = Remark::FIRST_VIP as u32;
            let first_civ = Remark::FIRST_CIVILIAN as u32;

            // Mismatch diagnostics: the `SPEECH_SCRIPT` flag flips the
            // prefix from "AI error" to "Script error" so QA can tell
            // script-driven mis-tags from engine-side bugs.
            let prefix = if flags.contains(SpeechFlags::SCRIPT) {
                "Script error"
            } else {
                "AI error"
            };
            let (group, excl_id) = if remark_u32 >= first_vip {
                if !snap.is_vip {
                    if snap.is_soldier {
                        tracing::warn!(
                            target: "ai_speech_mismatch",
                            "{}: Trying to play VIP remark [{}] for non-VIP soldier at ({},{})",
                            prefix,
                            snap.remark.speech(),
                            snap.position.x as u16,
                            snap.position.y as u16,
                        );
                    } else {
                        tracing::warn!(
                            target: "ai_speech_mismatch",
                            "{}: Trying to play VIP remark [{}] for non-VIP civilian at ({},{})",
                            prefix,
                            snap.remark.speech(),
                            snap.position.x as u16,
                            snap.position.y as u16,
                        );
                    }
                    blocked_mytalk.push((snap.entity_id, snap.flags));
                    continue;
                }
                (ExclamationGroup::Vip, (remark_u32 - first_vip) as u16)
            } else if remark_u32 >= first_civ {
                if snap.is_soldier {
                    tracing::warn!(
                        target: "ai_speech_mismatch",
                        "{}: Trying to play civilian remark [{}] for soldier at ({},{})",
                        prefix,
                        snap.remark.speech(),
                        snap.position.x as u16,
                        snap.position.y as u16,
                    );
                    blocked_mytalk.push((snap.entity_id, snap.flags));
                    continue;
                }
                (ExclamationGroup::Civilian, (remark_u32 - first_civ) as u16)
            } else {
                if !snap.is_soldier && !snap.is_vip {
                    tracing::warn!(
                        target: "ai_speech_mismatch",
                        "{}: Trying to play soldier remark [{}] for civilian at ({},{})",
                        prefix,
                        snap.remark.speech(),
                        snap.position.x as u16,
                        snap.position.y as u16,
                    );
                    blocked_mytalk.push((snap.entity_id, snap.flags));
                    continue;
                }
                // The soldier-remark fall-through passes
                // `EXCLAMATION_CIVILIAN`: soldiers play soldier remarks
                // via the civilian sound bank — a quirk of the original
                // engine; `EXCLAMATION_SOLDIER` is declared but never
                // used.
                (ExclamationGroup::Civilian, remark_u32 as u16)
            };

            // SPEECH_CYCLE_3_VARIANTS round-robins the variant index
            // across the three VIP-remark recordings so repeated lines
            // don't play the same sample twice.  The counter is shared
            // across all NPCs, stored on `AiGlobalState`.
            let variant = if flags.contains(SpeechFlags::CYCLE_3_VARIANTS) {
                self.ai_global.current_speech_variant =
                    (self.ai_global.current_speech_variant + 1) % 3;
                self.ai_global.current_speech_variant as i32
            } else {
                -1
            };

            // Emergency speech interrupts the actor's current line
            // before starting the replacement: if the actor is already
            // speaking and `SPEECH_EMERGENCY` is set, call
            // `stop_exclamation(self)` and then continue.
            if flags.contains(SpeechFlags::EMERGENCY) {
                self.pending_side_effects
                    .sounds
                    .push(super::SoundCommand::StopExclamation {
                        actor_id: snap.entity_id,
                    });
                self.sound_sim
                    .playing_exclamations
                    .retain(|p| p.actor_id != snap.entity_id.index());
            }

            // Queue the exclamation — drained by `flush_sound_queue` after
            // the tick body has finished so a rollback replay does not
            // stack-play duplicate audio.
            self.pending_side_effects
                .sounds
                .push(super::SoundCommand::Exclamation {
                    group,
                    profile_id: snap.speech_id,
                    exclamation_id: excl_id,
                    variant,
                    position: snap.position,
                    actor_id: Some(snap.entity_id),
                });
            // Schedule the deterministic MYTALK finish from the
            // host-populated sample-duration table. Missing samples use
            // the original zero-length completion path; sound backend
            // presence is deliberately not part of sim state.
            let duration = super::exclamation_duration_frames(
                &assets.exclamation_durations,
                group,
                snap.speech_id,
                excl_id,
            );
            self.sound_sim
                .playing_exclamations
                .push(crate::sound::PlayingExclamation {
                    actor_id: snap.entity_id.index(),
                    exclamation_id: excl_id as u32,
                    finish_frame: self.frame_counter + duration,
                });
            tracing::trace!(
                npc = snap.entity_id.index(),
                ?snap.remark,
                profile_id = snap.speech_id,
                excl_id,
                ?group,
                "speech dispatched"
            );

            // `forbid_remark`: auto-forbid based on remark type.
            // EntityId serves the same role as `creation_order`.
            Self::auto_forbid_remark(
                &mut self.ai_global.forbidden_remarks,
                snap.remark,
                snap.speech_id,
                snap.entity_id.index() as u16,
                snap.is_soldier,
                current_frame,
            );

            // Add to screen remarks (HUD subtitle display) with a
            // 100-frame timer.
            self.ai_global.screen_remarks.push(crate::ai::ScreenRemark {
                timer: 100,
                prefix: snap.profile_name.clone(),
                remark: snap.remark,
            });

            // Store flags for MYTALK callback when sound finishes (Phase 3).
            accepted_mytalk.push((snap.entity_id, snap.flags));
        }

        // Push blocked-remark MYTALK self-stimuli immediately:
        // `inform_ai_on_finished_remark` fires synchronously when
        // `say()` is blocked.
        for (entity_id, flags_bits) in blocked_mytalk {
            let flags = SpeechFlags::from_bits_truncate(flags_bits);
            let event = if flags.contains(SpeechFlags::MYTALK_1) {
                Some(StimulusType::EventMyTalk1)
            } else if flags.contains(SpeechFlags::MYTALK_2) {
                Some(StimulusType::EventMyTalk2)
            } else if flags.contains(SpeechFlags::MYTALK_3) {
                Some(StimulusType::EventMyTalk3)
            } else if flags.contains(SpeechFlags::MYTALK_0) {
                Some(StimulusType::EventMyTalk0)
            } else {
                None
            };
            // Blocked remarks never reached the sound manager, so
            // SoundIsFinished will not fire for them — we must clear
            // `current_remark` here or the `already speaking?` guard
            // would stay latched forever.
            if let Some(entity) = self.entities.get_mut(entity_id)
                && let Some(ai) = entity.ai_controller_mut()
            {
                ai.current_remark = Remark::TheSoundOfSilence;
                ai.current_remark_flags = 0;
                if let Some(stimulus_type) = event {
                    ai.pending_self_stimuli.push(stimulus_type);
                }
            }
        }

        // Store accepted flags for MYTALK callback on sound completion,
        // and latch `speech_in_flight` so the next pass through
        // `process_npc_speech` skips this NPC while the sound plays.
        for (entity_id, flags_bits) in accepted_mytalk {
            if let Some(entity) = self.entities.get_mut(entity_id)
                && let Some(ai) = entity.ai_controller_mut()
            {
                ai.pending_mytalk_flags = flags_bits;
                ai.speech_in_flight = true;
            }
        }

        // ── Phase 3: drain finished exclamations ────────────────
        // `sound_is_finished` callback: clear current_remark and fire
        // the MYTALK event (`inform_ai_on_finished_remark`).
        for &(actor_handle, _excl_id) in &self.sound_sim.finished_exclamations {
            if let Some(actor_id) = self.entities.id_at_legacy_slot(actor_handle)
                && let Some(entity) = self.entities.get_mut(actor_id)
            {
                // PC branch: nothing to do here — the C++ "currently
                // speaking" suppression that consumed sound-finished
                // events was already dead in legacy and has been
                // dropped from the Rust port.
                if matches!(entity, crate::element::Entity::Pc(_)) {
                    continue;
                }
                let npc = match entity {
                    crate::element::Entity::Soldier(s) => &mut s.npc,
                    crate::element::Entity::Civilian(c) => &mut c.npc,
                    _ => continue,
                };
                if let Some(ai) = npc.ai_brain.base_mut() {
                    ai.current_remark = Remark::TheSoundOfSilence;
                    ai.current_remark_flags = 0;
                    ai.speech_in_flight = false;

                    // Fire MYTALK callback based on stored flags.
                    let flags = SpeechFlags::from_bits_truncate(ai.pending_mytalk_flags);
                    ai.pending_mytalk_flags = 0;
                    let event = if flags.contains(SpeechFlags::MYTALK_1) {
                        Some(StimulusType::EventMyTalk1)
                    } else if flags.contains(SpeechFlags::MYTALK_2) {
                        Some(StimulusType::EventMyTalk2)
                    } else if flags.contains(SpeechFlags::MYTALK_3) {
                        Some(StimulusType::EventMyTalk3)
                    } else if flags.contains(SpeechFlags::MYTALK_0) {
                        Some(StimulusType::EventMyTalk0)
                    } else {
                        None
                    };
                    if let Some(stimulus_type) = event {
                        ai.pending_self_stimuli.push(stimulus_type);
                    }
                }
            }
        }
    }

    /// Per-tick decay + eviction of the screen-remark HUD overlay list.
    /// The timer half of `display_screen_remarks`: each entry's timer
    /// is decremented and entries whose timer reaches zero are
    /// dropped.  Without this the list grows unbounded for the
    /// lifetime of the mission (one entry per accepted remark).  The
    /// rendering half lives in `hud_text::render_screen_remarks`.
    pub(super) fn tick_screen_remarks(&mut self) {
        self.ai_global.screen_remarks.retain_mut(|r| {
            r.timer = r.timer.saturating_sub(1);
            r.timer > 0
        });
    }

    /// Auto-forbid a remark after speaking, with per-remark duration and scope.
    fn auto_forbid_remark(
        forbidden_remarks: &mut Vec<crate::ai::ForbiddenRemark>,
        remark: crate::ai::Remark,
        speech_id: u32,
        guy_index: u16,
        is_soldier: bool,
        current_frame: u32,
    ) {
        use crate::ai::{ForbiddenRemark, Remark, RemarkTargetFlags};
        use crate::parameters_ai::{
            AI_DRUNKEN_REMARK_FORBIDDEN_TIME, AI_REMARK_FORBIDDEN_TIME,
            AI_SHORT_REMARK_FORBIDDEN_TIME,
        };

        let push = |list: &mut Vec<ForbiddenRemark>, frames: i32, scope: RemarkTargetFlags| {
            list.push(ForbiddenRemark {
                remark,
                flags: scope.bits(),
                speech_id,
                guy_index,
                bad_guy: is_soldier,
                forbidden_till_frame: current_frame + frames as u32,
            });
        };

        match remark {
            // Never forbid — one-shot dialogue remarks.
            // These are used inside scripted conversations where a
            // second line in the same window must still play; forbidding
            // them would break multi-line officer/charly/beggar dialogs
            // and civ/vip wounded/dies pairs.
            Remark::Dies
            | Remark::Strangled
            | Remark::CivWounded
            | Remark::CivDies
            | Remark::VipWounded
            | Remark::VipDies
            | Remark::BadExcuse
            | Remark::CivBeggarBegging
            | Remark::CivBeggarGivesInfo
            | Remark::CivBeggarWantsMore
            | Remark::CivBeggarGivesLastInfo
            | Remark::CivBeggarThanx
            | Remark::OfficerStopsPatrol
            | Remark::OfficerStartsPatrol
            | Remark::OfficerAsksWhatsup
            | Remark::OfficerAsksWhere
            | Remark::OfficerEndsConversation
            | Remark::OfficerCallsSoldier
            | Remark::OfficerSendsOutSoldier
            | Remark::OfficerCallsGroup
            | Remark::OfficerSendsOutGroup
            | Remark::OfficerSendsOutGroupForCharly
            | Remark::OfficerRebukesCharly
            | Remark::OfficerRebukesCharlyEnd
            | Remark::OfficerGivesAttackOrder
            | Remark::OfficerSeesBrawl
            | Remark::OfficerEndsBrawl
            | Remark::GiveOrReceiveOrder
            | Remark::CallsOfficer
            | Remark::TellsOfficerBody
            | Remark::TellsOfficerEnemy
            | Remark::TellsOfficerOther
            | Remark::TellsOfficerCharlyAway
            | Remark::TellsOfficerWhere
            | Remark::AwaitsOrders
            | Remark::TellsOfficerNothing
            | Remark::CharlyDefendsHimself
            | Remark::MissesCharly
            | Remark::DidntFindCharly
            | Remark::FoundCharly
            | Remark::SendsCharlyToOfficer => {}

            // Short forbidden time.
            Remark::Wounded => {
                push(
                    forbidden_remarks,
                    AI_SHORT_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::THIS_TYPE,
                );
            }

            // Civilian sees body/dead body: ALL_NPC scope.
            Remark::CivSeesBody | Remark::CivSeesDeadBody => {
                push(
                    forbidden_remarks,
                    AI_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::ALL_NPC,
                );
            }

            // Drunken: double forbid — type + personal.
            Remark::Drunken => {
                push(
                    forbidden_remarks,
                    AI_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::THIS_TYPE,
                );
                push(
                    forbidden_remarks,
                    AI_DRUNKEN_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::THIS_GUY,
                );
            }

            // Standard forbidden time for everything else.
            _ => {
                push(
                    forbidden_remarks,
                    AI_REMARK_FORBIDDEN_TIME,
                    RemarkTargetFlags::THIS_TYPE,
                );
            }
        }
    }

    pub(super) fn tick_enemy_ai(&mut self, assets: &LevelAssets) {
        if self.actors_frozen() || self.ai_global.freeze {
            return;
        }
        let scratch = self.build_sim_scratch(assets);
        self.ai_global.same_frame_target_claims.clear();

        // Rebuild the per-tick handle → entity view map *before* the
        // detection pass starts firing stimuli into NPC Think() calls.
        // Every `AiContext` built in this method and its callees
        // picks up the refreshed map via
        // `scratch.ai_entity_views.clone()`.
        // ── 1. Build one immutable per-tick AI world view. ────────
        // Snapshot construction does not dispatch behavior. The phase calls
        // below remain in the original soldier/NPC Hourglass order.
        let world = self.tick_enemy_ai_build_world_view(assets);

        if world.pcs.is_empty() {
            return;
        }

        // ── 2a. Blip detection (reveal shadows). ────────────────
        self.tick_enemy_ai_blip_detection(assets, &world);

        // ── 2e. Shared acoustic-detection pass. ──────────────────
        // The hearing branch of `refresh_detection` plus
        // `update_hearing` — runs for every NPC (civilians +
        // Lacklandist soldiers), independent of the soldier-only
        // visual loop below.
        self.tick_enemy_ai_acoustic_detection(&world);

        // ── 3. Per-enemy RefreshDetection loop. ──────────────────
        let (transitions, out_of_view_dispatches) =
            self.tick_enemy_ai_refresh_detection(assets, &scratch, &world);

        // ── 3b. Royalist detection — reveal blipped enemies. ────
        self.tick_enemy_ai_royalist_detection(assets, &world);

        // ── 3c. Per-NPC non-Enemy detection. ────────────────────
        // The per-`type` outer loop arms of `refresh_detection` for
        // the five non-Enemy buckets — Body / Object / Friend /
        // MissedFriend / Beggar.  Builds a per-tick target map first
        // so each NPC's pass can dereference target metadata without
        // re-borrowing `self.entities`.
        self.tick_enemy_ai_refresh_per_type_detection(assets, &world);

        // ── 4. Log + pursue + alert nearby allies ───────────────
        self.tick_enemy_ai_alert_allies(&transitions);

        // ── 4b. Lost-sight EVENT_OUTOFVIEW dispatch. ───────────────
        self.tick_enemy_ai_dispatch_out_of_view(out_of_view_dispatches, &world.pcs);

        // Commit detection-local presentation state. Normal timer polling is
        // deliberately not part of this pass; NPC::Hourglass polls it only
        // after ambush, busy/ladder, lock gating, and The16thFrame.
        self.tick_enemy_ai_commit_detection_transitions(transitions);

        // ── 6c. Process pending AI swordfight requests. ─────────
        self.tick_enemy_ai_drain_swordfight_requests(assets);

        // ── 6d. Drain pending stimuli ────────────────────────────
        self.tick_enemy_ai_drain_pending_stimuli(assets, &scratch);
        self.ai_global.same_frame_target_claims.clear();

        // Sword strikes are launched by `engine::melee::tick_enemy_sword_attacks`.
        // Keep this AI pass to target selection, pursuit, and swordfight
        // requests; applying direct damage here would bypass the
        // wait-timer + interaction sequence timing.
    }

    /// Per-NPC drain for all `pending_*` flags on [`AiController`] that
    /// mutate engine state (launch sequences / orders, toggle attentive
    /// mode, fire cross-NPC stimuli, etc.).  Extracted from the global
    /// post-Think drain loop so the same body can also run synchronously
    /// right after each [`Self::dispatch_filtered_stimulus`] call via
    /// [`Self::dispatch_think_with_drain`] — matching `think()`
    /// semantics where handler side effects (`launch_sequence`,
    /// `set_attentive_mode`, `face`, …) are immediate.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn drain_pending_for_npc(
        &mut self,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        let scratch = self.build_sim_scratch(assets);
        // Drain pending_halt FIRST so the actor's in-progress sequence
        // (typically a Move element while running toward the target) is
        // torn down before any subsequent intent (e.g.
        // `pending_enter_swordfight`) launches a new sequence.
        // `begin_swordfight` / `break_macro` callers call
        // `stop_all() → halt() → stop(PREFERENCE)` inline before
        // `launch_sequence_element(EnterSwordfight)`.
        //
        // Without this ordering, `enter_swordfight`'s
        // `pathfinder.cancel_requests_for` (a no-op post-refactor) and
        // local `clear_path` leave the orphaned Move sequence in
        // InProgress state.  An in-flight path response then
        // `try_dispatch_move_path`s onto the actor a few ticks later,
        // restoring `active_movement` and re-driving the run animation
        // — the visual "stuck in running pose" symptom.
        let take_halt = {
            let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = s.npc.ai_brain.base_mut() else {
                return;
            };
            let h = ai.pending_halt;
            ai.pending_halt = false;
            h
        };
        if take_halt {
            self.halt_actor(npc_id);
        }

        // Read+clear `pending_stop_menace` separately — keeps the giant
        // tuple below from growing yet another slot.
        let stop_menace = {
            let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = s.npc.ai_brain.base_mut() else {
                return;
            };
            let v = ai.pending_stop_menace;
            ai.pending_stop_menace = false;
            v
        };

        // Same shape as `pending_stop_menace` above — prepend a
        // `Command::LowerShield` element ahead of the move when the
        // actor is in any shield action-state, matching the
        // any-shield-action-state arm in `go_to`.
        let lower_shield = {
            let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = s.npc.ai_brain.base_mut() else {
                return;
            };
            let v = ai.pending_lower_shield;
            ai.pending_lower_shield = false;
            v
        };

        // Read and clear pending flags.
        let (
            quit,
            enter,
            enter_jl,
            stop_target,
            set_principal,
            friend_target_swap,
            shoot,
            do_focus,
            do_focus_point,
            do_unfocus,
            set_dir,
            deactivate,
            broadcast_panic,
            launch_cmds,
            look_sidewards,
            add_detectables,
            delete_detectables,
            delete_detectable_entities,
            slowly_open_eyes,
            launch_on_target,
            launch_sequences,
            set_posture,
        ) = {
            let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = s.npc.ai_brain.base_mut() else {
                return;
            };
            let q = ai.pending_quit_swordfight;
            let e = ai.pending_enter_swordfight.take();
            let e_jl = ai.pending_enter_swordfight_jump_line.take();
            let st = ai.pending_stop_target.take();
            let p = ai.pending_set_principal.take();
            let friend_swap = ai.pending_friend_primary_target_swap.take();
            let shoot = ai.pending_shoot_target.take();
            let focus = ai.pending_focus.take();
            let focus_point = ai.pending_focus_point.take();
            let uf = ai.pending_unfocus;
            let sd = ai.pending_set_direction_instantly.take();
            let deact = ai.pending_deactivate;
            let panic = ai.pending_broadcast_panic;
            let launch_cmds = std::mem::take(&mut ai.pending_launch_commands);
            let launch_on_target = std::mem::take(&mut ai.pending_launch_on_target);
            let launch_sequences = std::mem::take(&mut ai.pending_launch_sequences);
            let look = ai.pending_look_sidewards.take();
            let add_det = std::mem::take(&mut ai.pending_add_detectables);
            let del_det = std::mem::take(&mut ai.pending_delete_detectables);
            let del_det_entity = std::mem::take(&mut ai.pending_delete_detectable_entity);
            let open_eyes = ai.pending_slowly_open_eyes;
            let posture = ai.pending_posture.take();
            ai.pending_quit_swordfight = false;
            ai.pending_unfocus = false;
            ai.pending_deactivate = false;
            ai.pending_broadcast_panic = false;
            ai.pending_slowly_open_eyes = false;
            (
                q,
                e,
                e_jl,
                st,
                p,
                friend_swap,
                shoot,
                focus,
                focus_point,
                uf,
                sd,
                deact,
                panic,
                launch_cmds,
                look,
                add_det,
                del_det,
                del_det_entity,
                open_eyes,
                launch_on_target,
                launch_sequences,
                posture,
            )
        };

        // Process quit_swordfight.
        if quit {
            self.quit_swordfight(assets, npc_id);
        }

        // Process stop_menace — the explicit `STOP_MENACE` element
        // prepend in `go_to`.  Launching a `Command::StopMenace`
        // element here lets the per-element dispatch in `tick.rs`
        // queue `TRANSITION_MENACING_WAITING_SWORD` then
        // `TRANSITION_LOWERING_SWORD` before the move that
        // `launch_pending_orders_for_npc` is about to launch starts.
        if stop_menace {
            let elem = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::StopMenace,
                Some(npc_id),
            );
            self.launch_element(elem);
        }

        // Process lower_shield — the explicit `LOWER_SHIELD` element
        // prepend in `go_to`.  Launching a `Command::LowerShield`
        // element here lets `dispatch_lower_shield` queue the
        // `LoweringShield` order so the shield arm completes before
        // `launch_pending_orders_for_npc` runs the move.
        if lower_shield {
            let elem = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::LowerShield,
                Some(npc_id),
            );
            self.launch_element(elem);
        }

        // Process pending `stop()` on a different entity — the
        // `primary_target.stop()` call inside `begin_swordfight`.  The
        // default `stop()` uses `Normal` priority.  Drained before
        // `enter_swordfight` so the target's in-flight Move element is
        // torn down before the engine-side ENTER_SWORDFIGHT sequence
        // runs.
        if let Some(target_handle) = stop_target {
            let target_id = EntityId::Pc(crate::entity_id::PcId(target_handle));
            self.stop_owner(target_id, crate::sequence::SequencePriority::Normal);
        }

        // Process enter_swordfight.  Two shapes:
        //   * Engage(target) — engagement against a specific opponent.
        //     Run the full `enter_swordfight_with_jump_line` path so the
        //     opponent lists, jump-line links, and
        //     `prepare_to_enter_swordfight` `stop(Preference)` cascade
        //     fire correctly.
        //   * RaiseSword — sword pose without engagement. `go_to`'s
        //     `GOTO_SWORD` arm, `AttackingApproachToObserve`, and
        //     menace-effect-of-hit need a sword pose held without an
        //     active fight.
        if let Some(request) = enter {
            match request {
                crate::ai::EnterSwordfightRequest::RaiseSword => {
                    let elem = crate::sequence::SequenceElement::new_generic(
                        1,
                        crate::element::Command::EnterSwordfight,
                        Some(npc_id),
                    );
                    self.launch_element(elem);
                }
                crate::ai::EnterSwordfightRequest::Engage(target_handle) => {
                    let target_id = EntityId::Pc(crate::entity_id::PcId(target_handle));
                    let aggressor_jl = enter_jl.and_then(crate::jump_line::JumpLineIndex::new);
                    self.enter_swordfight_with_jump_line(
                        assets,
                        npc_id,
                        target_id,
                        false,
                        aggressor_jl,
                    );
                }
            }
        }

        // Process set_as_new_principal_opponent.
        if let Some(opponent_handle) = set_principal {
            let opponent_id = EntityId::Pc(crate::entity_id::PcId(opponent_handle));
            self.set_as_new_principal_opponent(assets, npc_id, opponent_id);
        }

        // Process friend primary-target swap.  The reference calls
        // `friend.set_primary_target(primary_target)` directly on the
        // other soldier when the swap heuristic fires; we hand it off
        // here so both soldiers are updated consistently after their
        // AI ticks ran.
        if let Some((friend_id, new_target)) = friend_target_swap
            && let Some(Entity::Soldier(s)) = self.entities.get_mut(friend_id)
            && let Some(friend_ai) = s.npc.ai_brain.base_mut()
        {
            friend_ai.primary_target = new_target;
        }

        // Process pending bow shot.
        if let Some(target_handle) = shoot {
            let target_id = EntityId::Pc(crate::entity_id::PcId(target_handle));
            self.shoot_bow_at(assets, npc_id, target_id);
        }

        // Process pending focus / focus_point / unfocus — the
        // `focus(primary_target)` / `focus(position&)` / `focus(NULL)`
        // calls.  Each explicit channel "consumes" the primary_target
        // edge by stamping `last_synced_focus_target = primary_target`,
        // so `refresh_npc_views` sees no edge and does not auto-revert
        // the explicit focus state next tick.  This is what makes
        // patterns like rider-charge passing (`focus(NULL)` while
        // `primary_target` stays set) and `battle_decisions` entry
        // honour the synchronous ordering even though the channel
        // itself is deferred.
        let mut focus_channel_fired = false;
        if let Some(target_handle) = do_focus
            && let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
        {
            crate::ai_vision::focus_entity(
                &mut s.npc,
                EntityId::Pc(crate::entity_id::PcId(target_handle)),
            );
            focus_channel_fired = true;
        }

        if let Some(point) = do_focus_point
            && let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
        {
            crate::ai_vision::focus_point(
                &mut s.npc,
                crate::coordinates::MapPoint::new(point.x, point.y),
            );
            focus_channel_fired = true;
        }

        if do_unfocus && let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) {
            crate::ai_vision::unfocus(&mut s.npc);
            focus_channel_fired = true;
        }

        if focus_channel_fired
            && let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
            && let Some(ai) = s.npc.ai_brain.base_mut()
        {
            ai.last_synced_focus_target = (ai.primary_target != 0).then_some(ai.primary_target);
        }

        // Process pending SlowlyOpenEyes — `slowly_open_eyes` sets
        // `view_radius = 5`, points `view_radius_goal` at the engine's
        // standard view radius, switches `eye_status` to
        // `ViewconeGrow`, and marks `view_transition`.  The
        // `ViewconeGrow` branch of `refresh_view` then ramps the cone
        // back open at 8 units/frame.
        if slowly_open_eyes {
            let standard = self.standard_view_polygon_radius;
            if let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) {
                s.npc.view_transition = true;
                s.npc.view_radius = 5;
                s.npc.view_radius_base = 5;
                s.npc.view_radius_goal = standard;
                s.npc.eye_status = crate::element::EyeStatus::ViewconeGrow;
            }
        }

        // Process pending set_direction_instantly.
        if let Some(dir) = set_dir
            && let Some(entity) = self.entities.get_mut(npc_id)
        {
            entity.position_iface_mut().set_direction_instantly(
                crate::position_interface::Direction::from_raw(dir as i32),
            );
        }

        // Launch any pending AI orders (Face/Turn, GoTo movement,
        // generic animation) BEFORE draining
        // `pending_set_attentive_mode`.  `face` / `go_to` call
        // `launch_sequence` inline inside the think handler, so by the
        // time `set_attentive_mode(true)` fires `ENTER_ATTENTIVE_MODE`
        // at `postpone_everything_but_injuries`, there's already an
        // active sequence at `Normal` priority to preempt — whose
        // `send_condolation_card` dispatches `think(EVENT_DONE)`
        // re-entrantly, advancing
        // `AttackingReactiontimeTurning → AttackingReactiontime` the
        // same frame as `EVENT_VIEW`.
        self.launch_pending_orders_for_npc(npc_id);

        // Process pending `set_attentive_mode(target, fast_officer)`:
        //   * Flip `will_be_attentive = target`.
        //   * If `target != attentive`, book an
        //     `ENTER_/LEAVE_ATTENTIVE_MODE` sequence element whose
        //     `translate` inserts the appropriate
        //     TRANSITION_WAITING_UPRIGHT_WAITING_ALERTED (or reverse /
        //     officer) animation.
        // The Rust port doesn't have a full sequence-command path for
        // these yet, so we book the transition animation directly via
        // `active_ai_anim` when the soldier is idle; the `execute`
        // side-effect handler (animation.rs) flips `attentive` when
        // the transition reaches DONE/TERMINATED.  When the soldier
        // is NOT idle (already has `active_ai_anim` or `combat_anim`)
        // we snap the flags instead so the next combat decision sees
        // the correct state — the "Consider as done" branch.
        let attentive_request = {
            let mut take = None;
            if let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
                && let Some(base) = s.npc.ai_brain.base_mut()
            {
                take = base.pending_set_attentive_mode.take();
            }
            take
        };
        if let Some((target, fast_officer)) = attentive_request {
            // Route the request through the sequence pipeline
            // so the order-driven animation handler runs —
            // identical to any other `set_soldier_attentive_mode`
            // call.  `translate` handles the "posture isn't upright" /
            // "already busy" gating when the dispatcher runs next
            // tick; snapping the flag immediately here would race
            // that.
            self.set_soldier_attentive_mode(npc_id, target, fast_officer);
        }

        // Process pending `SetGuardedPC` — `set_guarded_pc`.  The AI
        // wrote its own `guarded_pc` field already; here we flip the
        // reciprocal `pc.guard` on the old and new target PCs.
        let guard_delta = if let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
            && let Some(base) = s.npc.ai_brain.base_mut()
        {
            base.pending_set_guarded_pc.take()
        } else {
            None
        };
        if let Some((old_pc, new_pc)) = guard_delta {
            // Clear `pc.guard` on the old target
            // (`guarded_pc.set_guard(NULL)`).
            if old_pc != 0
                && let Some(Entity::Pc(pc)) = self.entities.get_mut(EntityId::Pc(PcId(old_pc)))
            {
                pc.pc.guard = None;
            }
            // Set `pc.guard` on the new target
            // (`guarded_pc.set_guard(self)`).  Asserts `is_in_coma()`
            // on the PC; the only caller already gates on the coma
            // check in the `AttackingApproachingSleepingEnemy`
            // handler, so skip the redundant debug_assert here.
            if new_pc != 0
                && let Some(Entity::Pc(pc)) = self.entities.get_mut(EntityId::Pc(PcId(new_pc)))
            {
                pc.pc.guard = Some(npc_id);
            }
        }

        // Process pending entity deactivation (merry man leaving map).
        // Equivalent to `set_active(false)`.
        if deactivate && let Some(entity) = self.entities.get_mut(npc_id) {
            entity.element_data_mut().active = false;
            tracing::debug!(
                npc = npc_id.index(),
                "Deactivated entity (merry man left map)"
            );
        }

        // Process pending `set_reported_to_officer(flag)` — the
        // `charly.set_reported_to_officer(false)` call inside
        // `missed_charly_alert`.  Writes the other NPC's
        // `EnemyAi::reported_to_officer` flag.
        let reported_updates = if let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
            && let Some(ai) = s.npc.ai_brain.base_mut()
        {
            std::mem::take(&mut ai.pending_set_reported_to_officer)
        } else {
            Vec::new()
        };
        for (target_handle, value) in reported_updates {
            let target_id = EntityId::Soldier(SoldierId(target_handle));
            let Some(Entity::Soldier(s)) = self.entities.get_mut(target_id) else {
                continue;
            };
            if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                enemy_ai.reported_to_officer = value;
            }
        }

        // Process pending bow-ammo refill — the
        // `set_ammo_amount(BOW, MAX_NPC_ARROWS)` call inside
        // `fleeing_run_for_arrow_reserves`.
        {
            let refill = if let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
                && let Some(ai) = s.npc.ai_brain.base_mut()
            {
                let r = ai.pending_refill_bow_ammo;
                ai.pending_refill_bow_ammo = false;
                r
            } else {
                false
            };
            if refill && let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) {
                s.npc.number_of_arrows = crate::parameters_ai::MAX_NPC_ARROWS as u16;
            }
        }

        // Process pending archery-sector release — the
        // `set_my_archery_sector(NULL)` call queued from
        // `EnemyAi::set_state` when the soldier leaves an archer-wait
        // substate.  Decrement the owner counter on the current
        // archery sector and clear the index.  The companion
        // `pending_release_shooting_point` carries the prior shooting
        // point's `(sector, point)` so we can also run the
        // `set_my_shooting_point(NULL)` `set_owner(NULL)` write here —
        // the AI layer already cleared its own `my_shooting_point`
        // field synchronously in `set_state`.
        {
            let release = if let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
                && let Some(enemy) = s.npc.ai_brain.enemy_mut()
            {
                let sector = if std::mem::take(&mut enemy.pending_release_archery_sector) {
                    enemy.my_archery_sector.take()
                } else {
                    None
                };
                let point = enemy.pending_release_shooting_point.take();
                (sector, point)
            } else {
                (None, None)
            };
            if let (_, Some((sec_idx, pt_idx))) = release
                && let Some(sector) = self.ai_global.archery_sectors.get_mut(sec_idx as usize)
                && let Some(pt) = sector.points.get_mut(pt_idx as usize)
            {
                pt.owner = None;
            }
            if let (Some(idx), _) = release
                && let Some(sector) = self.ai_global.archery_sectors.get_mut(idx as usize)
            {
                sector.decrement_owner_counter();
            }
        }

        // Process pending UnalertAllNearCharlySeekers — walks all
        // soldier NPCs in the same camp and for each candidate that
        //   - is alive / active / not the seeker / not the charly,
        //   - passes the rank/antagonist guard
        //     `(seeker_rank == OFFICER || cs != antagonist)`,
        //   - and detects either charly or self within 180°,
        // dispatches `CALL_CHARLY_IS_BACK` carrying charly's handle.
        // The pending field's payload selects either self or an
        // explicit Charly handle.
        let unalert = if let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
            && let Some(ai) = s.npc.ai_brain.base_mut()
        {
            let u = ai.pending_unalert_near_charly_seekers;
            ai.pending_unalert_near_charly_seekers = None;
            u
        } else {
            None
        };
        if let Some(target_charly) = unalert {
            let (my_camp, my_pos, my_rank, my_antagonist) = match self.get_entity(npc_id) {
                Some(Entity::Soldier(s)) => (
                    Some(s.soldier.cached_camp),
                    Some(s.element.position_map()),
                    s.npc
                        .ai_brain
                        .enemy()
                        .map(|e| e.soldier_profile_rank)
                        .unwrap_or(crate::profiles::ProfileRank::None),
                    s.npc.ai_brain.base().map(|b| b.antagonist).unwrap_or(0),
                ),
                _ => (None, None, crate::profiles::ProfileRank::None, 0),
            };
            let charly_handle = match target_charly {
                crate::ai::CharlySeekerTarget::SelfNpc => npc_id.index(),
                crate::ai::CharlySeekerTarget::Npc(handle) => handle,
            };
            let charly_pos = self
                .entities
                .id_at_legacy_slot(charly_handle)
                .and_then(|charly_id| self.entities.get(charly_id))
                .map(|e| {
                    let pm = e.element_data().position_map();
                    crate::ai::Position {
                        x: pm.x,
                        y: pm.y,
                        sector: e.element_data().sector(),
                        level: e.element_data().layer(),
                    }
                });
            if let (Some(camp), Some(my_pos), Some(charly_pos)) = (my_camp, my_pos, charly_pos) {
                let my_pos_pi = crate::ai::Position {
                    x: my_pos.x,
                    y: my_pos.y,
                    sector: None,
                    level: 0,
                };
                let vr = if self.standard_view_polygon_radius > 0 {
                    self.standard_view_polygon_radius as f32
                } else {
                    ai_vision::DEFAULT_VIEW_RADIUS as f32
                };
                let sq_vr = vr * vr;
                let charly_is_self = charly_handle == npc_id.index();
                for other_id in self.entities.npc_ids().collect::<Vec<_>>() {
                    if other_id == npc_id {
                        continue;
                    }
                    if other_id.index() == charly_handle {
                        continue;
                    }
                    // Rank/antagonist guard:
                    //   `rank == Officer || other != antagonist`.
                    if my_rank != crate::profiles::ProfileRank::Officer
                        && other_id.index() == my_antagonist
                    {
                        continue;
                    }
                    let (eligible, other_pos, other_dir, other_able) = {
                        let Some(Entity::Soldier(os)) = self.entities.get(other_id) else {
                            continue;
                        };
                        let pm = os.element.position_map();
                        let pos = crate::ai::Position {
                            x: pm.x,
                            y: pm.y,
                            sector: os.element.sector(),
                            level: os.element.layer(),
                        };
                        let able =
                            os.element.active && !os.human.unconscious && os.npc.life_points > 0;
                        (
                            os.soldier.cached_camp == camp
                                && os.npc.life_points > 0
                                && os.element.active,
                            pos,
                            os.element.direction() as u16,
                            able,
                        )
                    };
                    if !eligible {
                        continue;
                    }
                    // Cheap cull: at least one of (charly, me) within
                    // view-radius square distance.
                    // `is_detecting_180_degrees` would handle this
                    // internally; keep the gate for consistency with
                    // the prior implementation.
                    let dx_c = other_pos.x - charly_pos.x;
                    let dy_c = other_pos.y - charly_pos.y;
                    let dx_m = other_pos.x - my_pos.x;
                    let dy_m = other_pos.y - my_pos.y;
                    if dx_c * dx_c + dy_c * dy_c > sq_vr && dx_m * dx_m + dy_m * dy_m > sq_vr {
                        continue;
                    }
                    // Facing cone:
                    //   is_detecting_180_degrees(charly)
                    //   || (charly != self && is_detecting_180_degrees(self))
                    let detects_charly = other_able
                        && crate::ai_enemy::detects_position_180_raw(
                            other_pos, other_dir, charly_pos, sq_vr,
                        );
                    let detects_me_branch = !charly_is_self
                        && other_able
                        && crate::ai_enemy::detects_position_180_raw(
                            other_pos, other_dir, my_pos_pi, sq_vr,
                        );
                    if !(detects_charly || detects_me_branch) {
                        continue;
                    }
                    let stimulus = crate::ai::Stimulus::with_human(
                        crate::ai::StimulusType::CallCharlyIsBack,
                        charly_handle,
                    );
                    let other_ctx = {
                        let Some(entity) = self.entities.get(other_id) else {
                            continue;
                        };
                        build_ai_context_from_entity(
                            entity,
                            self.frame_counter,
                            None,
                            self.weather.is_forest_level,
                            self.standard_view_polygon_radius,
                            &scratch.ai_entity_views,
                            &scratch.ai_sight_obstacles,
                            &self.fast_grid,
                            &assets.hiking_paths,
                            &self.ai_global.all_soldier_handles,
                        )
                    };
                    let tick_data = self.build_npc_tick_data(other_id, &scratch, assets);
                    self.dispatch_filtered_stimulus(
                        assets, other_id, &stimulus, &other_ctx, &tick_data,
                    );
                }
            }
        }

        // Process pending civilian panic broadcast.
        // `nearby_civilians_panic` iterates nearby civilians within
        // the standard view radius (aspect-ratio box +
        // `is_detecting_360_degrees`) and dispatches EVENT_PANIC.
        // Both this enemy-broadcast path and the sword-attack call
        // site funnel through the same helper, since both use
        // EVENT_PANIC with the same filter.
        if broadcast_panic {
            self.nearby_civilians_panic(assets, npc_id);
        }

        // Process pending launch commands — create and launch
        // sequence elements for commands the AI wants to execute.
        for cmd in launch_cmds {
            let elem = crate::sequence::SequenceElement::new(1, cmd, Some(npc_id));
            self.launch_element(elem);
        }

        // Sequence commands the AI wants to launch on *another*
        // entity (e.g. soldier forcing a beggar to stand up).
        // Equivalent to a `launch_sequence_element(cmd,
        // other_actor)` call as used by the enemy beggar-identify
        // cascade.
        for (target_handle, cmd) in launch_on_target {
            let target_id = EntityId::Pc(crate::entity_id::PcId(target_handle));
            let elem = crate::sequence::SequenceElement::new(1, cmd, Some(target_id));
            self.launch_element(elem);
        }

        // Full sequences the AI wants to launch verbatim — the
        // `launch_sequence(SEQ_INFO, sequence)` calls inside AI
        // handlers (e.g. the officer's turn/gather/point alert
        // sequence).
        for seq in launch_sequences {
            self.launch_sequence(seq);
        }

        // Process pending LookSidewards — build a one- or two-element
        // sequence of LookLeft / LookRight / LeanOut commands and
        // launch it.
        if let Some(dir) = look_sidewards {
            use crate::ai::LookDirection;
            use crate::element::Command;
            let cmds: &[Command] = match dir {
                LookDirection::Left => &[Command::LookLeft],
                LookDirection::Right => &[Command::LookRight],
                LookDirection::LeftRight => &[Command::LookLeft, Command::LookRight],
                LookDirection::RightLeft => &[Command::LookRight, Command::LookLeft],
                LookDirection::Down => &[Command::LeanOut],
            };
            tracing::trace!(
                npc = npc_id.index(),
                ?dir,
                ?cmds,
                "launching look-sidewards sequence"
            );
            // `look_sidewards` calls `focus(NULL)` before allocating
            // the sequence so the soldier's gaze drops its lock for
            // the head-turn animation.  Centralise it here instead of
            // patching every caller.
            if let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) {
                crate::ai_vision::unfocus(&mut s.npc);
            }
            let mut seq = crate::sequence::Sequence::new();
            for (i, cmd) in cmds.iter().enumerate() {
                let elem =
                    crate::sequence::SequenceElement::new((i as u16) + 1, *cmd, Some(npc_id));
                seq.append_element(elem);
            }
            self.launch_sequence(seq);
        }

        // Process pending "strip beggar from every NPC" requests:
        //   delete_detectable_for_all_npc(stimulus.human, BEGGAR);
        // Fired from the `EventSeesBeggar` handler in `ai_enemy.rs`
        // once a seek-area soldier has claimed the PC-beggar via
        // `beggars_to_control`, so every other soldier's BEGGAR list
        // drops the PC and stops firing duplicate `EventSeesBeggar`
        // stimuli on subsequent frames.
        let delete_beggar_requests: Vec<EntityId> = {
            let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = s.npc.ai_brain.base_mut() else {
                return;
            };
            std::mem::take(&mut ai.pending_delete_beggar_for_all_npc)
        };
        for beggar_id in delete_beggar_requests {
            self.delete_beggar_detectable_for_all_npc(beggar_id);
        }

        // Process pending detectable modifications.
        if !add_detectables.is_empty()
            || !delete_detectables.is_empty()
            || !delete_detectable_entities.is_empty()
        {
            // Resolve target classification for each ENEMY-arm push
            // so the `add_detectable` filter can run.  Resolved
            // up-front to avoid borrowing `self.entities` mutably
            // while we read target metadata from it.
            use crate::element::DetectableType;
            let enemy_target_info: Vec<Option<(bool, bool, crate::element_kinds::Camp, bool)>> =
                add_detectables
                    .iter()
                    .map(|(eid, dt)| {
                        if *dt != DetectableType::Enemy {
                            return None;
                        }
                        self.get_entity(*eid)
                            .map(|e| (e.is_pc(), e.is_soldier(), e.camp(), e.is_human()))
                    })
                    .collect();

            if let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) {
                let npc_camp = s.soldier.cached_camp;
                let npc_is_soldier = true; // dispatch already filtered to Soldier
                // Delete all detectables of specified types.
                for det_type in &delete_detectables {
                    let idx = *det_type as usize;
                    if idx < s.npc.detectable_lists.len() {
                        s.npc.detectable_lists[idx].clear();
                    }
                }
                // Per-entity deletes: `delete_detectable(entity, type)`
                // drops a single (element, type) entry, leaving
                // siblings of the same type alone.
                for (entity_id, det_type) in &delete_detectable_entities {
                    let idx = *det_type as usize;
                    if idx < s.npc.detectable_lists.len() {
                        s.npc.detectable_lists[idx].retain(|d| d.element != Some(*entity_id));
                    }
                }
                // Add new detectables.
                for ((entity_id, det_type), tgt) in
                    add_detectables.iter().zip(enemy_target_info.iter())
                {
                    let idx = *det_type as usize;
                    if idx >= s.npc.detectable_lists.len() {
                        continue;
                    }
                    // ENEMY-arm filter — drop pushes that fail the
                    // per-NPC camp/rank arm so a Royalist soldier
                    // never tracks a PC and a Lacklandist civilian
                    // never tracks a Royalist soldier.
                    if *det_type == DetectableType::Enemy {
                        let Some((tgt_pc, tgt_soldier, tgt_camp, tgt_human)) = *tgt else {
                            continue;
                        };
                        if !tgt_human {
                            continue;
                        }
                        if !crate::ai_detectable_filter::should_add_enemy_detectable(
                            npc_camp,
                            npc_is_soldier,
                            tgt_pc,
                            tgt_soldier,
                            tgt_camp,
                        ) {
                            continue;
                        }
                    }
                    // Don't add duplicates.
                    let already = s.npc.detectable_lists[idx]
                        .iter()
                        .any(|d| d.element == Some(*entity_id));
                    if !already {
                        s.npc.detectable_lists[idx].push(crate::element::Detectable {
                            element: Some(*entity_id),
                            detectable_type: *det_type,
                            ..Default::default()
                        });
                    }
                }
            }
        }

        // Process pending `RestoreDetectableObjects` request — walks
        // every active engine element, filters to `Ale` (always) and
        // `Coin` (gated by `!knocked_out_in_money_fight`), and
        // appends each survivor to the NPC's `DETECTABLE_OBJECT` list
        // when not already present.  Fired from the enemy
        // `EVENT_FITAGAIN` / `SleepingUnconscious` handler.
        let restore_request = {
            let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = s.npc.ai_brain.base_mut() else {
                return;
            };
            if ai.pending_restore_detectable_objects {
                ai.pending_restore_detectable_objects = false;
                Some(ai.knocked_out_in_money_fight)
            } else {
                None
            }
        };
        if let Some(knocked_out_in_money_fight) = restore_request {
            use crate::element::{DetectableType, EntityId};
            use crate::element_kinds::ObjectType;
            let det_idx = DetectableType::Object as usize;
            let mut to_add: Vec<EntityId> = Vec::new();
            for (entity_id, entity) in self.entities.objects() {
                if !entity.is_active() {
                    continue;
                }
                let Some(obj) = entity.object_data() else {
                    continue;
                };
                let restore_this = match obj.object_type {
                    ObjectType::Ale => true,
                    ObjectType::Coin => !knocked_out_in_money_fight,
                    _ => false,
                };
                if restore_this {
                    to_add.push(entity_id.into());
                }
            }
            if !to_add.is_empty()
                && let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
                && det_idx < s.npc.detectable_lists.len()
            {
                for elem_id in to_add {
                    let already = s.npc.detectable_lists[det_idx]
                        .iter()
                        .any(|d| d.element == Some(elem_id));
                    if !already {
                        s.npc.detectable_lists[det_idx].push(crate::element::Detectable {
                            element: Some(elem_id),
                            detectable_type: DetectableType::Object,
                            ..Default::default()
                        });
                    }
                }
            }
        }

        // Process pending `ForgetAllNearbyCoins` request — the first
        // half of `forget_all_nearby_coins`: walk the
        // `DETECTABLE_OBJECT` list and drop every coin entry whose
        // referenced element is within Chebyshev 500 of `pos`.  The
        // second half (`other_seen_money.clear()`) is performed
        // synchronously on the AI side in
        // `EnemyAi::forget_all_nearby_coins`.
        let forget_pos = {
            let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = s.npc.ai_brain.base_mut() else {
                return;
            };
            ai.pending_forget_nearby_coins.take()
        };
        if let Some(pos) = forget_pos {
            use crate::element::DetectableType;
            use crate::element_kinds::ObjectType;
            const NEARBY_COIN_DISTANCE: f32 = 500.0;
            let det_idx = DetectableType::Object as usize;
            // Snapshot the candidate element ids first so we can read
            // `entities` immutably while iterating, then mutate the
            // detectable list in a second pass.
            let mut to_remove: Vec<crate::element::EntityId> = Vec::new();
            if let Some(Entity::Soldier(s)) = self.entities.get(npc_id)
                && det_idx < s.npc.detectable_lists.len()
            {
                for det in &s.npc.detectable_lists[det_idx] {
                    let Some(elem_id) = det.element else {
                        continue;
                    };
                    let Some(elem) = self.entities.get(elem_id) else {
                        continue;
                    };
                    let Some(obj) = elem.object_data() else {
                        continue;
                    };
                    if obj.object_type != ObjectType::Coin {
                        continue;
                    }
                    let elem_pos = elem.element_data().position_map();
                    let dx = (elem_pos.x - pos.x).abs();
                    let dy = (elem_pos.y - pos.y).abs();
                    if dx.max(dy) < NEARBY_COIN_DISTANCE {
                        to_remove.push(elem_id);
                    }
                }
            }
            if !to_remove.is_empty()
                && let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
                && det_idx < s.npc.detectable_lists.len()
            {
                s.npc.detectable_lists[det_idx]
                    .retain(|d| d.element.is_none_or(|id| !to_remove.contains(&id)));
            }
        }

        // Process pending SetPosture request.  Like the
        // `set_posture(Sitting/Leisure)` calls in the reference.
        // The move-box recomputation in
        // `PositionInterface::set_posture` is skipped here because
        // the engine stores posture on the element-data struct and
        // the move box is reshaped lazily elsewhere — this matches
        // every other posture write in the codebase (e.g.
        // `abilities.rs` `CarryingCorpse`, `melee.rs` knock-out
        // paths).
        if let Some(p) = set_posture
            && let Some(entity) = self.entities.get_mut(npc_id)
        {
            entity.set_posture(p);
        }

        // Process pending BlinkEnemy(NULL) request — clear the
        // seen_now / seen_last_frame flags on every enemy detectable
        // so the next detection pass treats anyone still in the cone
        // as a "first-seen" edge and re-issues EVENT_VIEW.
        let (blink_all, blink_specific) = {
            let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = s.npc.ai_brain.base_mut() else {
                return;
            };
            let b = ai.pending_blink_all_enemies;
            ai.pending_blink_all_enemies = false;
            let specific = std::mem::take(&mut ai.pending_blink_enemy_specific);
            (b, specific)
        };
        if blink_all && let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) {
            let idx = crate::element::DetectableType::Enemy as usize;
            if let Some(list) = s.npc.detectable_lists.get_mut(idx) {
                for det in list.iter_mut() {
                    det.seen_now = false;
                    det.seen_last_frame = false;
                }
            }
        }
        // Single-target arm of `blink_enemy(enemy)`.  Unlike the
        // all-enemies sweep above this only clears the detection
        // latch for a specific target, so when that target is still
        // in the cone the next detection pass re-fires `EVENT_VIEW`
        // against it as a "first-seen" edge.
        if !blink_specific.is_empty()
            && let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id)
        {
            let idx = crate::element::DetectableType::Enemy as usize;
            if let Some(list) = s.npc.detectable_lists.get_mut(idx) {
                for det in list.iter_mut() {
                    if det.element.is_some_and(|e| blink_specific.contains(&e)) {
                        det.seen_now = false;
                        det.seen_last_frame = false;
                    }
                }
            }
        }

        // Process pending `EnemyInHouseAlert` request.
        //
        // Orchestrator walks the building's occupant list, sorts by
        // camp, dispatches `panic()` to civilians, and calls
        // `init_battle_before_door` on the outnumbered side.  Both
        // the panic side-effect and the door-battle orchestration
        // (`init_battle_before_door` + `send_before_door_to_fight`
        // in `engine/soldier_helpers.rs`) are wired below.
        let in_house_alert = {
            let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = s.npc.ai_brain.base_mut() else {
                return;
            };
            let v = ai.pending_enemy_in_house_alert;
            ai.pending_enemy_in_house_alert = false;
            v
        };
        if in_house_alert {
            self.dispatch_enemy_in_house_alert(npc_id, assets);
        }

        // Drain any pending panic request from the enemy AI — the
        // analogue of the civilian-side drain that runs inside
        // `nearby_civilians_panic`.  Without this, an EnemyAi that
        // pushes a `PanicRequest` (e.g. from the fleeing arm of
        // `think_alerting_event(EVENT_VIEW)` outdoors) stays wedged
        // in `FleeingPanic` with no door picked.
        let ctx_for_panic = {
            let Some(entity) = self.entities.get(npc_id) else {
                return;
            };
            let entity_sector = entity.element_data().sector();
            let building_sector = self.entity_building_sector(entity_sector);
            let Some(entity) = self.entities.get(npc_id) else {
                return;
            };
            build_ai_context_from_entity(
                entity,
                self.frame_counter,
                building_sector,
                self.weather.is_forest_level,
                self.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.fast_grid,
                &assets.hiking_paths,
                &self.ai_global.all_soldier_handles,
            )
        };
        self.process_pending_begin_panic_for(npc_id, &ctx_for_panic);
        self.process_pending_panic_seek_fallback_for(npc_id, &ctx_for_panic);

        // Drain any pending script-driven SeekArea request.  Matches
        // the immediate `start_think(NO_EVENT); seek_area(...);
        // end_think()` block inside `set_ai_state(STATE_SEEKING)`.
        //
        // Only pay the surrounding battle-context cost when the
        // request exists. Keep the cheap pre-check here so the common
        // drain pass does not rebuild full per-NPC tick data for
        // every soldier just to discover
        // `pending_script_seek_area == None`.
        let has_script_seek = self
            .entities
            .get(npc_id)
            .and_then(|entity| entity.ai_controller())
            .is_some_and(|ai| ai.pending_script_seek_area.is_some());
        if has_script_seek {
            let tick_for_seek = self.build_npc_tick_data(npc_id, &scratch, assets);
            self.process_pending_script_seek_area_for(npc_id, &ctx_for_panic, &tick_for_seek);
        }
    }

    /// Alert nearby allied soldiers to look at a position.
    ///
    /// Iterates every soldier in the same camp as `source`, and for
    /// each one in `STATE_DEFAULT` / `STATE_WONDERING` /
    /// `SEEKING_JUST_WATCHING` within `radius` of the source, fires
    /// the `CALL_LOOKTHERE → call_look_there_standard_procedure`
    /// transition:
    ///
    ///   * `StopAll()` — clear active path
    ///   * `SetState(STATE_WONDERING, SUBSTATE_WONDERING_WATCHING)`
    ///   * `seek_position = where`
    ///   * `face(seek_position)` — turn to look at the alert
    ///   * `LaunchTimer(100)`
    ///
    /// `radius` is `VIEW_LOOK_THERE_RADIUS = 100` for vision-based
    /// alerts and 200 for noise-based ones.
    pub(crate) fn hey_folks_look_there(&mut self, source: EntityId, pos: MapPoint, radius: f32) {
        let (source_camp, source_pos) = {
            let Some(Entity::Soldier(src)) = self.entities.get(source) else {
                return;
            };
            (src.soldier.cached_camp, src.element.position_map())
        };

        let radius_sq = radius * radius;
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for npc_id in npc_ids {
            if npc_id == source {
                continue;
            }
            // Check eligibility (immut borrow).
            let eligible = {
                let Some(Entity::Soldier(s)) = self.entities.get(npc_id) else {
                    continue;
                };
                if s.soldier.cached_camp != source_camp {
                    continue;
                }
                if s.npc.life_points <= 0 || s.human.unconscious {
                    continue;
                }
                // Filter: STATE_DEFAULT / STATE_WONDERING /
                // SUBSTATE_SEEKING_JUST_WATCHING.
                let state_ok = matches!(
                    s.npc.ai_state(),
                    crate::ai::AiState::Default | crate::ai::AiState::Wondering
                ) || matches!(
                    s.npc.ai_substate(),
                    crate::ai::Substate::SeekingJustWatching
                        | crate::ai::Substate::SeekingJustWatchingSidewards
                );
                if !state_ok {
                    continue;
                }
                // Range check (square distance to avoid sqrt).
                let p = s.element.position_map();
                let dx = source_pos.x - p.x;
                let dy = source_pos.y - p.y;
                dx * dx + dy * dy < radius_sq
            };
            if !eligible {
                continue;
            }

            // Apply the CallLookThereStandardProcedure transition.
            if let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) {
                // Face toward the seek position via
                // `vector_to_sector_0_to_15_iso`.
                let p = s.element.position_map();
                let dx = pos.x - p.x;
                let dy = pos.y - p.y;
                s.element.set_direction_instantly(
                    crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
                );

                // `stop_all()` — decouple the actor from its Move
                // element; priority arbitration with the next launched
                // sequence tears down the orphaned Move.
                s.actor.active_movement.clear();
                s.actor.action_state = crate::element::ActionState::Waiting;

                // SetState(WONDERING, WONDERING_WATCHING)
                if let Some(ai) = s.npc.ai_brain.base_mut() {
                    ai.set_ai_state(crate::ai::AiState::Wondering);
                    ai.current_substate = crate::ai::Substate::WonderingWatching;
                    ai.seek_position = crate::ai::Position {
                        x: pos.x,
                        y: pos.y,
                        sector: None,
                        level: 0,
                    };
                    ai.launch_timer(100, self.frame_counter);
                }
            }
        }
    }

    /// Make nearby civilians panic.
    ///
    /// Iterates every civilian within `view_radius` of `source`,
    /// dispatches `EventPanic` through the civilian's
    /// [`crate::ai_friendly::FriendlyAi::think`] — which sets
    /// `FleeingPanic` and records a [`crate::ai::PanicRequest`] on the
    /// AI base — then drains the request against
    /// `ai_global.door_seek_infos` so a matching door gets picked and
    /// a `GoTo(door_in)` order queued.
    /// Orchestrate a building-wide enemy alert.
    ///
    /// Walks the building's occupant list, splits it into royalists /
    /// lacklandists / civilians, panics the civilians, and — if both
    /// camps are present — stages the outnumbered side to flee the
    /// building while the stronger side pursues
    /// (`init_battle_before_door` follow-on).
    ///
    /// `send_before_door_to_fight` is ported as
    /// [`EngineInner::send_before_door_to_fight`], and the
    /// `init_battle_before_door` orchestration — pick nearest door,
    /// compute defender/attacker positions, fan out
    /// `send_before_door_to_fight` per occupant — is ported as
    /// [`EngineInner::init_battle_before_door`] and called below.
    #[tracing::instrument(level = "trace", skip_all, fields(source = source.index()))]
    pub(crate) fn dispatch_enemy_in_house_alert(&mut self, source: EntityId, assets: &LevelAssets) {
        // Find the source NPC's building sector.
        let source_sector = {
            let Some(entity) = self.entities.get(source) else {
                return;
            };
            let sector = entity.element_data().sector();
            match self.entity_building_sector(sector) {
                Some(_) => sector, // real building
                None => return,    // source left the building already
            }
        };

        let building_sector_num = match source_sector {
            Some(s) => u32::from(s),
            None => return,
        };

        // Look up the matching House to get the occupant list.
        let Some(house) = self
            .ai_global
            .houses
            .iter()
            .find(|h| h.sector_index == building_sector_num)
        else {
            return;
        };
        let door_indices = house.door_indices.clone();
        let occupant_ids = house.occupant_ids.clone();

        // Split occupants into royalists / lacklandists / civilians,
        // skipping dead and unconscious.  PCs count as royalists.
        let mut royalist_ids: Vec<EntityId> = Vec::new();
        let mut lacklandist_ids: Vec<EntityId> = Vec::new();
        let mut civilian_ids: Vec<EntityId> = Vec::new();
        for &eid in &occupant_ids {
            let Some(entity) = self.entities.get(eid) else {
                continue;
            };
            match entity {
                Entity::Soldier(s) => {
                    if s.npc.life_points <= 0 || s.human.unconscious {
                        continue;
                    }
                    match s.soldier.cached_camp {
                        crate::element::Camp::Royalists => royalist_ids.push(eid),
                        crate::element::Camp::Lacklandists => lacklandist_ids.push(eid),
                        _ => {}
                    }
                }
                Entity::Civilian(c) => {
                    if c.npc.life_points <= 0 || c.human.unconscious {
                        continue;
                    }
                    civilian_ids.push(eid);
                }
                Entity::Pc(pc) if pc.pc.life_points > 0 && !pc.human.unconscious => {
                    royalist_ids.push(eid);
                }
                _ => {}
            }
        }

        // No battle unless both camps present.
        if royalist_ids.is_empty() || lacklandist_ids.is_empty() {
            return;
        }

        // Every live civilian panics.
        let panic_runs = crate::parameters_ai::AI_STANDARD_PANIC_RUNS as u8;
        for civ_id in civilian_ids {
            self.process_building_civilian_panic(assets, civ_id, panic_runs);
        }

        // Outnumbered side flees; the stronger side pursues.
        let (fleeing, pursuing) = if royalist_ids.len() > lacklandist_ids.len() {
            (lacklandist_ids, royalist_ids)
        } else {
            (royalist_ids, lacklandist_ids)
        };

        self.init_battle_before_door(&door_indices, &fleeing, &pursuing);

        tracing::debug!(
            source = source.index(),
            building = building_sector_num,
            fleeing = fleeing.len(),
            pursuing = pursuing.len(),
            "EnemyInHouseAlert: civilians panicked, door-battle dispatched"
        );
    }

    /// Make a single civilian panic from the building alert.
    /// Equivalent to the inline
    /// `civilians[i].panic(AI_STANDARD_PANIC_RUNS)` loop body in
    /// `enemy_in_house_alert`.
    #[tracing::instrument(level = "trace", skip_all, fields(civ = civ_id.index(), runs))]
    fn process_building_civilian_panic(
        &mut self,
        assets: &LevelAssets,
        civ_id: EntityId,
        runs: u8,
    ) {
        let scratch = self.build_sim_scratch(assets);
        let ctx = {
            let Some(entity) = self.entities.get(civ_id) else {
                return;
            };
            let entity_sector = entity.element_data().sector();
            let building_sector = self.entity_building_sector(entity_sector);
            let Some(entity) = self.entities.get(civ_id) else {
                return;
            };
            build_ai_context_from_entity(
                entity,
                self.frame_counter,
                building_sector,
                self.weather.is_forest_level,
                self.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.fast_grid,
                &assets.hiking_paths,
                &self.ai_global.all_soldier_handles,
            )
        };

        if let Some(Entity::Civilian(c)) = self.entities.get_mut(civ_id)
            && let Some(friendly_ai) = c.npc.ai_brain.friendly_mut()
        {
            let was_already_fleeing = matches!(
                friendly_ai.base.current_substate,
                crate::ai::Substate::FleeingPanic | crate::ai::Substate::FleeingRunToDoor
            );
            friendly_ai.base.lasting_panic_runs = runs;
            friendly_ai.base.directed_panic = false;
            friendly_ai.base.current_state = crate::ai::AiState::Fleeing;
            friendly_ai.base.current_substate = crate::ai::Substate::FleeingPanic;
            friendly_ai.base.pending_begin_panic = Some(crate::ai::PanicRequest {
                center: None,
                runs,
                alert: crate::ai::AlertLevel::Red,
                is_new_panic: !was_already_fleeing,
            });
        }

        // Drain the PanicRequest so a door gets picked and GoTo fires.
        self.process_pending_begin_panic_for(civ_id, &ctx);
        self.process_pending_panic_seek_fallback_for(civ_id, &ctx);
    }

    #[tracing::instrument(level = "trace", skip_all, fields(source = source.index()))]
    pub(crate) fn nearby_civilians_panic(&mut self, assets: &LevelAssets, source: EntityId) {
        let scratch = self.build_sim_scratch(assets);
        let view_radius = if self.standard_view_polygon_radius > 0 {
            self.standard_view_polygon_radius as f32
        } else {
            ai_vision::DEFAULT_VIEW_RADIUS as f32
        };
        // `nearby_civilians_panic` builds an aspect-ratio-stretched
        // axis-aligned box (radius, radius * ASPECT_RATIO) around
        // self, then walks every NPC asking:
        //   is_civilian() && box.is_inside(p) && p->is_detecting_360_degrees(self)
        // The second-stage detection uses the civilian's upright eye
        // point, the source actor's detection point, the civilian's
        // live view radius, and opaque 3D LOS.
        let radius_y = view_radius * crate::position_interface::ASPECT_RATIO;

        let (source_pos, source_detection_point) = {
            let Some(entity) = self.entities.get(source) else {
                return;
            };
            // Source must be IsActiveAndOutsideBuilding for
            // IsDetecting360Degrees to ever return true.
            if !entity.element_data().active {
                return;
            }
            if self
                .entity_building_sector(entity.element_data().sector())
                .is_some()
            {
                return;
            }
            let Some(detection_point) = entity.compute_detection_point() else {
                return;
            };
            (entity.element_data().position_map(), detection_point)
        };

        let panic_center = crate::ai::Position {
            x: source_pos.x,
            y: source_pos.y,
            sector: None,
            level: 0,
        };

        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        // Clone the Arc-shared snapshot so the per-civilian filter can
        // call `los_clear` without holding an immutable borrow on
        // `self.ai_global` across the later `process_pending_*` mutable
        // borrows.
        let obstacles_owned = scratch.ai_sight_obstacles.clone();
        for npc_id in npc_ids {
            let obstacles = obstacles_owned.list();
            let eligible = {
                let Some(entity) = self.entities.get(npc_id) else {
                    continue;
                };
                let Entity::Civilian(c) = entity else {
                    continue;
                };
                if c.npc.life_points <= 0 || c.human.unconscious {
                    continue;
                }
                // IsActiveAndOutsideBuilding on the civilian.
                if !c.element.active {
                    continue;
                }
                if self.entity_data_inside_building(&c.element) {
                    continue;
                }
                let p = c.element.position_map();
                let dx = source_pos.x - p.x;
                let dy = source_pos.y - p.y;
                // Aspect-ratio bounding box: |dx| <= r,
                // |dy| <= r * ASPECT_RATIO.
                if dx.abs() > view_radius || dy.abs() > radius_y {
                    continue;
                }
                let Some(viewer_eye) =
                    entity.compute_eyes_point(Some(crate::element::Posture::Upright))
                else {
                    continue;
                };
                // IsDetecting360Degrees(actor) stretched-Y 3D distance
                // gate: civilian upright eye to source detection point,
                // clamped by the civilian's live real view radius.
                let dx = source_detection_point.x - viewer_eye.x;
                let dy = (source_detection_point.y - viewer_eye.y)
                    * crate::position_interface::INVERSE_ASPECT_RATIO;
                let dz = source_detection_point.z - viewer_eye.z;
                let sq_view_radius = {
                    let radius = c.npc.view_radius as f32;
                    radius * radius
                };
                if dx * dx + dy * dy + dz * dz > sq_view_radius {
                    continue;
                }
                crate::sight_obstacle::is_reachable_3d(
                    obstacles,
                    [viewer_eye.x, viewer_eye.y, viewer_eye.z],
                    [
                        source_detection_point.x,
                        source_detection_point.y,
                        source_detection_point.z,
                    ],
                    crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
                )
            };
            if !eligible {
                continue;
            }

            // Build per-civilian AiContext and dispatch EVENT_PANIC.
            let ctx = {
                let Some(entity) = self.entities.get(npc_id) else {
                    continue;
                };
                build_ai_context_from_entity(
                    entity,
                    self.frame_counter,
                    None,
                    self.weather.is_forest_level,
                    self.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.fast_grid,
                    &assets.hiking_paths,
                    &self.ai_global.all_soldier_handles,
                )
            };

            let stimulus = crate::ai::Stimulus::with_position(
                crate::ai::StimulusType::EventPanic,
                panic_center,
            );
            // Civilian EventPanic: FriendlyAi — no combat tick data
            // consumed, stub is correct.
            let tick_data = AiPerTickData::stub();
            self.dispatch_filtered_stimulus(assets, npc_id, &stimulus, &ctx, &tick_data);

            // Drain the resulting `PanicRequest`: find a door, or fall
            // back to the `FleeingPanic` run-segment state machine.
            self.process_pending_begin_panic_for(npc_id, &ctx);
            self.process_pending_panic_seek_fallback_for(npc_id, &ctx);
        }
    }

    /// Re-issue an in-flight patrol `GoTo` so a freshly-changed
    /// `default_path_walking_flags` (typically RUN ↔ WALK from
    /// the `SetPathWalkingStyle` script native) takes effect
    /// immediately rather than at the next waypoint pickup.
    /// The relaunch tail of `set_path_walking_flags`:
    ///
    /// ```ignore
    /// if has_patrol_path && substate in {DefaultGotoRoute, DefaultEnroute} {
    ///     let mut flags = default_path_walking_flags;
    ///     if !will_stop_at_next_waypoint() { flags |= GotoFlags::DONT_STOP; }
    ///     go_to(current_waypoint_position, flags);
    /// }
    /// ```
    pub(crate) fn relaunch_path_at_new_speed(&mut self, assets: &LevelAssets, npc_id: EntityId) {
        let scratch = self.build_sim_scratch(assets);
        // Re-check the gate (state may have changed between the
        // native pushing the deferred command and us draining it).
        let (has_path, substate) = {
            let Some(entity) = self.entities.get(npc_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller() else {
                return;
            };
            (ai.has_patrol_path, ai.current_substate)
        };
        if !has_path
            || !matches!(
                substate,
                crate::ai::Substate::DefaultGotoRoute | crate::ai::Substate::DefaultEnroute
            )
        {
            return;
        }

        // Look up the current waypoint position from the level's
        // hiking paths.  Bail if the AI has no patrol path or the
        // waypoint index is out of range — both indicate a desync
        // that the relaunch can't repair on its own.
        let waypoint_position = {
            let Some(entity) = self.entities.get(npc_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller() else {
                return;
            };
            let Some(path) = ai.patrol_path.as_ref() else {
                return;
            };
            let Some(wp) = path.current_waypoint(&assets.hiking_paths) else {
                return;
            };
            crate::ai::Position {
                x: wp.x as f32,
                y: wp.y as f32,
                sector: crate::position_interface::SectorHandle::new(wp.sector),
                level: wp.level,
            }
        };

        // Build the per-tick AiContext for `go_to` (mirrors how the
        // panic / patrol-coordination paths build it).
        let ctx = {
            let Some(entity) = self.entities.get(npc_id) else {
                return;
            };
            let entity_sector = entity.element_data().sector();
            let building_sector = self.entity_building_sector(entity_sector);
            build_ai_context_from_entity(
                entity,
                self.frame_counter,
                building_sector,
                self.weather.is_forest_level,
                self.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.fast_grid,
                &assets.hiking_paths,
                &self.ai_global.all_soldier_handles,
            )
        };

        // Compute `WillStopAtNextWaypoint` and call `go_to`.
        let Some(entity) = self.entities.get_mut(npc_id) else {
            return;
        };
        let Some(ai) = entity.ai_controller_mut() else {
            return;
        };
        let will_stop = ai.will_stop_at_next_waypoint(&assets.hiking_paths);
        let mut flags = ai.default_path_walking_flags;
        if !will_stop {
            flags |= crate::ai::GotoFlags::DONT_STOP;
        }
        ai.go_to(waypoint_position, flags, &ctx);
    }

    /// Drain a queued [`PanicRequest`] on a single NPC.
    ///
    /// Called right after any `FriendlyAi::think` that could have
    /// pushed a panic request (the civilian EVENT_PANIC /
    /// EVENT_VIEW-from-swordfighting-soldier handlers).  The `panic`
    /// door-search + GoTo fall back:
    ///
    ///  * Walk `ai_global.door_seek_infos` for a `Building` door in a
    ///    *different* building from the actor, authorised for the
    ///    actor, and — when `directed` — pointing *away* from the
    ///    panic center.  Apply +500 sector-change / +300 layer-change
    ///    malus to the `MaxNorm` distance and pick the minimum.
    ///  * If found → `Substate::FleeingRunToDoor`, reset
    ///    `lasting_panic_runs`, issue a running `GoTo(door_in)` via
    ///    the AI base's `go_to` helper.
    ///  * If not found → stay in `Substate::FleeingPanic`, bump
    ///    `lasting_panic_runs` to `runs + 1`, and fire a self
    ///    `EventReachPoint` so the `think_expected_event_common_stuff`
    ///    panic-run branch picks a random escape vector next tick.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn process_pending_begin_panic_for(
        &mut self,
        npc_id: EntityId,
        ctx: &crate::ai::AiContext,
    ) {
        // Peel the request off the AI base.
        let Some(entity) = self.entities.get_mut(npc_id) else {
            return;
        };
        let Some(ai) = entity.ai_controller_mut() else {
            return;
        };
        let Some(request) = ai.pending_begin_panic.take() else {
            return;
        };

        // Resolve the actor's current building for the
        // "not this building" filter used by `GetNearestDoor`.
        let my_building = ctx.in_building.then_some(ctx.building_sector).flatten();
        let my_layer = ctx.position.level;

        // Pre-compute the set of house sector indices that contain a
        // PC (the `dangerous_house` set).  Snapshot it here so the
        // `pick_door` closure doesn't need to borrow `self.entities`
        // (which is re-borrowed mutably after door selection).
        let dangerous_house_sectors: std::collections::HashSet<u32> =
            if ctx.camp == crate::element::Camp::Lacklandists {
                self.ai_global
                    .houses
                    .iter()
                    .filter(|h| {
                        h.occupant_ids.iter().any(|&eid| {
                            matches!(self.entities.get(eid), Some(crate::element::Entity::Pc(_)))
                        })
                    })
                    .map(|h| h.sector_index)
                    .collect()
            } else {
                std::collections::HashSet::new()
            };

        // Pick the best door.  `directed` gates the dot-product
        // filter: when a panic center is known, first try to find a
        // door in the "away" half-plane; if none exists, fall back to
        // an undirected lookup (clearing `directed_panic`).
        let pick_door = |directed: bool| -> Option<(crate::ai::Position, u32)> {
            let mut best: Option<(crate::ai::Position, u32)> = None;
            for door in &self.ai_global.door_seek_infos {
                if !matches!(door.door_type, crate::gate::DoorType::Building) {
                    continue;
                }
                if !door.npc_villain_authorized_direct {
                    continue;
                }
                if my_building == crate::position_interface::SectorHandle::new(door.sector_in) {
                    continue;
                }
                let dx_door = door.point_out.x - ctx.position.x;
                let dy_door = door.point_out.y - ctx.position.y;
                if directed && let Some(center) = request.center {
                    let dx_flee = center.x - ctx.position.x;
                    let dy_flee = center.y - ctx.position.y;
                    if dx_door * dx_flee + dy_door * dy_flee >= 0.0 {
                        continue;
                    }
                }
                let mut distance = dx_door.abs().max(dy_door.abs()) as u32;
                if Some(door.sector_out) != ctx.position.sector.map(u16::from) {
                    distance = distance.saturating_add(500);
                }
                if door.layer_out != my_layer {
                    distance = distance.saturating_add(300);
                }
                if best.map(|(_, d)| distance < d).unwrap_or(true) {
                    // `dangerous_house` check.  A fleeing Lacklandist
                    // never runs into a building that already contains
                    // a PC; the gate is camp-gated so Royalist
                    // civilians (and all other camps) skip it.
                    if !dangerous_house_sectors.contains(&(door.sector_in as u32)) {
                        best = Some((door.position_in, distance));
                    }
                }
            }
            best
        };

        let directed_initial = request.center.is_some();
        let mut best = pick_door(directed_initial);
        // Directed → undirected door fallback.  If no door satisfies
        // the away-half-plane filter, retry with the filter dropped
        // and clear the directed-panic flag on the controller.
        let mut directed_after_door_pick = directed_initial;
        if best.is_none() && directed_initial {
            best = pick_door(false);
            directed_after_door_pick = false;
        }

        // Snapshot whether the entity is a civilian so we can pick
        // the right Say() remark after we re-borrow the AI base.
        let is_civilian = self
            .entities
            .get(npc_id)
            .map(|e| e.is_civilian())
            .unwrap_or(false);

        // Re-borrow the AI base for either branch.
        let Some(entity) = self.entities.get_mut(npc_id) else {
            return;
        };
        let Some(ai) = entity.ai_controller_mut() else {
            return;
        };

        // Sync `directed_panic` with the door-pick outcome
        // (`directed_panic = false` on the fallback path).
        ai.directed_panic = directed_after_door_pick;

        ai.break_macro();
        ai.set_transient_emoticon(crate::ai::EmoticonType::XMark, 0, ctx.frame);

        if let Some((door_in, _)) = best {
            // Door-found arm.
            if is_civilian {
                ai.say(crate::ai::Remark::CivPanic);
            }
            ai.set_ai_state(crate::ai::AiState::Fleeing);
            ai.current_substate = crate::ai::Substate::FleeingRunToDoor;
            ai.set_alert_status(request.alert);
            ai.lasting_panic_runs = 0;
            ai.go_to(door_in, crate::ai::GotoFlags::RUN, ctx);

            // Post-GoTo `couldnt_reachpoint` fallback.  Pathfinding
            // here runs asynchronously so the flag will usually only
            // be set from a prior tick's failed GoTo, but the retry
            // is kept for parity.  If the directed attempt was
            // unreachable, drop the dot-product filter and try again;
            // if even that fails, fall through to the no-door
            // branch.
            if ai.couldnt_reachpoint {
                ai.couldnt_reachpoint = false;
                if directed_after_door_pick && let Some((retry_door, _)) = pick_door(false) {
                    let Some(entity) = self.entities.get_mut(npc_id) else {
                        return;
                    };
                    let Some(ai) = entity.ai_controller_mut() else {
                        return;
                    };
                    ai.directed_panic = false;
                    ai.go_to(retry_door, crate::ai::GotoFlags::RUN, ctx);
                    if !ai.couldnt_reachpoint {
                        return;
                    }
                    ai.couldnt_reachpoint = false;
                    self.begin_panic_no_door_branch(npc_id, &request, ctx, is_civilian);
                    return;
                }
                self.begin_panic_no_door_branch(npc_id, &request, ctx, is_civilian);
            }
            return;
        }

        self.begin_panic_no_door_branch(npc_id, &request, ctx, is_civilian);
    }

    /// Drain a queued `pending_panic_seek_fallback` on a single NPC.
    ///
    /// `FLEEING_PANIC` / `EventCouldntReachPoint` fallback: the
    /// panic-run GoTo was blocked, so pick the nearest seek point
    /// (with a +1000 sector-change and +5000 fleeing-toward-source
    /// penalty applied by
    /// [`crate::ai::AiController::nearest_seek_point_to_flee`]) and
    /// GoTo it, with `RUN | DONT_STOP` mid-panic-run and plain `RUN`
    /// on the last segment.  If no seek point is in range, re-fire
    /// the self `EventReachPoint` for the emergency case
    /// fall-through.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn process_pending_panic_seek_fallback_for(
        &mut self,
        npc_id: EntityId,
        ctx: &crate::ai::AiContext,
    ) {
        let Some(entity) = self.entities.get_mut(npc_id) else {
            return;
        };
        let Some(ai) = entity.ai_controller_mut() else {
            return;
        };
        if !ai.pending_panic_seek_fallback {
            return;
        }
        ai.pending_panic_seek_fallback = false;

        let anchor = ai.nearest_seek_point_to_flee(
            &self.ai_global.seek_points,
            ctx.position,
            ctx.position.sector,
        );

        let Some(entity) = self.entities.get_mut(npc_id) else {
            return;
        };
        let Some(ai) = entity.ai_controller_mut() else {
            return;
        };

        match anchor {
            Some(idx) => {
                let dest = self.ai_global.seek_points[idx].position;
                let Some(entity) = self.entities.get_mut(npc_id) else {
                    return;
                };
                let Some(ai) = entity.ai_controller_mut() else {
                    return;
                };
                let mut flags = crate::ai::GotoFlags::RUN;
                if ai.lasting_panic_runs > 0 {
                    flags |= crate::ai::GotoFlags::DONT_STOP;
                }
                ai.go_to(dest, flags, ctx);
                if ai.couldnt_reachpoint {
                    // Emergency-case retry — decrement runs and
                    // self-fire `EventReachPoint` so the common-stuff
                    // state machine tries a new random direction on
                    // the next tick.
                    ai.couldnt_reachpoint = false;
                    ai.lasting_panic_runs = ai.lasting_panic_runs.saturating_sub(1);
                    ai.fire_self_stimulus(crate::ai::StimulusType::EventReachPoint);
                }
            }
            None => {
                // Emergency case — no seek point available, re-fire
                // reach-point so the common-stuff handler picks a
                // fresh random direction.
                ai.fire_self_stimulus(crate::ai::StimulusType::EventReachPoint);
            }
        }
    }

    /// No-door branch of `panic`.  Split out so the door-found
    /// branch can fall through on a post-GoTo unreachable-point
    /// error.
    fn begin_panic_no_door_branch(
        &mut self,
        npc_id: EntityId,
        request: &crate::ai::PanicRequest,
        ctx: &crate::ai::AiContext,
        is_civilian: bool,
    ) {
        let Some(entity) = self.entities.get_mut(npc_id) else {
            return;
        };
        let Some(ai) = entity.ai_controller_mut() else {
            return;
        };

        // If directed, OR in the "panic center is in front of me"
        // dot-product test so a center that has flipped in front
        // during a prior run still counts as a new panic.
        let mut is_new_panic = request.is_new_panic;
        if request.center.is_some() && !is_new_panic {
            let (dx_face, dy_face) = crate::element::direction_vector_16(ctx.direction as i16);
            let dx = ai.panic_center_x - ctx.position.x;
            let dy = ai.panic_center_y - ctx.position.y;
            if dx_face * dx + dy_face * dy > 0.0 {
                is_new_panic = true;
            }
        }

        if is_new_panic {
            // New panic — full side-effect set.
            ai.set_ai_state(crate::ai::AiState::Fleeing);
            ai.current_substate = crate::ai::Substate::FleeingPanic;
            ai.say(if is_civilian {
                crate::ai::Remark::CivPanic
            } else {
                crate::ai::Remark::Panic
            });
            ai.set_alert_status(request.alert);
            ai.lasting_panic_runs = request.runs.saturating_add(1);
            ai.first_try = true;
            ai.fire_self_stimulus(crate::ai::StimulusType::EventReachPoint);
        } else {
            // Not new: upgrade-only bump of `lasting_panic_runs`
            // (`if lasting_panic_runs < runs`).  No state change, no
            // `say()`, no self-fire.
            if ai.lasting_panic_runs < request.runs {
                ai.lasting_panic_runs = request.runs;
            }
        }
    }

    /// Drain a pending script-driven `SeekArea` request.  Consumes
    /// `AiController::pending_script_seek_area` set by
    /// `script_set_ai_state` when a script fires
    /// `SetAIState(actor, STATE_SEEKING)`.  Dispatches into
    /// `EnemyAi::seek_area` (soldier-only — `seek_area` is defined
    /// only on the soldier subtype).
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn process_pending_script_seek_area_for(
        &mut self,
        npc_id: EntityId,
        ctx: &crate::ai::AiContext,
        tick: &crate::ai::AiPerTickData,
    ) {
        let request = {
            let Some(entity) = self.entities.get_mut(npc_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            let Some(req) = ai.pending_script_seek_area.take() else {
                return;
            };
            req
        };

        let Some(Entity::Soldier(s)) = self.entities.get_mut(npc_id) else {
            return;
        };
        let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() else {
            // Non-enemy soldier (civilian-brained) — `seek_area` is
            // undefined on non-soldier brains, same as the original
            // assert.
            return;
        };
        enemy_ai.seek_area(
            request.center,
            request.radius,
            crate::ai_enemy::SeekFlags::empty(),
            crate::ai_enemy::UNDEFINED_DIRECTION,
            &mut self.ai_global,
            ctx,
            tick,
        );
    }

    // ─── Patrol coordination ───────────────────────────────────

    /// Per-frame patrol coordination tick.
    ///
    /// The chief-side patrol management of the base AI class:
    /// 1. **`instruct_patrol_direction_to_patrol_members` drain** —
    ///    applies any `CMD_PATROL_DIRECTION` broadcast queued by the
    ///    chief's macro VM to each minion synchronously.
    /// 2. **`initialize_patrol`** — build active patrol from
    ///    theoretical members (check state, sort by distance,
    ///    pair-swap) on the `needs_patrol_reinit` one-shot flag.
    /// 3. **`refresh_patrol`** — every frame record chief history,
    ///    every 8th frame compute formation positions and dispatch
    ///    `CALL_PATROL_COORDINATE` to each minion.
    ///
    /// `transform_patrol_ids_to_real_patrol` is no longer part of
    /// this tick — it lives in `EngineInner::init_one_ai`, invoked
    /// once at AI bootstrap.
    pub(super) fn tick_patrol_coordination(&mut self, assets: &LevelAssets) {
        use crate::ai::{AiState, Position, Stimulus, StimulusType, Substate};

        if self.actors_frozen() || self.ai_global.freeze {
            return;
        }
        let scratch = self.build_sim_scratch(assets);

        let frame = self.frame_counter;
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();

        // ── Phase 0: Drain pending CMD_PATROL_DIRECTION broadcasts ──
        // `instruct_patrol_direction_to_patrol_members` —
        // `CMD_PATROL_DIRECTION` calls this synchronously inside the
        // macro VM, but the Rust macro VM runs on the chief's
        // `AiController` without engine access — it queues the
        // directive and we drain it here before any other patrol
        // work, so the per-minion
        // `set_instructed_patrol_direction(direction, &ctx)` call sees
        // the same `current_substate` (and therefore the same
        // `face_to`-on-WAITING side effect).
        let mut pending_direction_broadcasts: Vec<(EntityId, u16)> = Vec::new(); // (minion, direction)
        for &npc_id in &npc_ids {
            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                continue;
            };
            let Some(direction) = ai.pending_patrol_direction_broadcast.take() else {
                continue;
            };
            for &member_id in &ai.patrol {
                if member_id != npc_id {
                    pending_direction_broadcasts.push((member_id, direction));
                }
            }
        }
        for (minion_id, direction) in pending_direction_broadcasts {
            let Some(entity) = self.entities.get_mut(minion_id) else {
                continue;
            };
            if !entity.is_active() || entity.is_dead() {
                continue;
            }
            let ctx = build_ai_context_from_entity(
                entity,
                frame,
                None,
                self.weather.is_forest_level,
                self.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.fast_grid,
                &assets.hiking_paths,
                &self.ai_global.all_soldier_handles,
            );
            if let Some(ai) = entity.ai_controller_mut() {
                ai.set_instructed_patrol_direction(direction, &ctx);
            }
        }

        // ── Phase 2: Snapshot NPC states ──
        // Needed for patrol initialization and missed-member checks.
        #[derive(Clone, Copy)]
        struct NpcSnap {
            position: Position,
            direction: u16,
            ai_state: AiState,
            is_alive: bool,
            is_active: bool,
            view_radius: u16,
            move_box: crate::coordinates::MoveBox,
            // Patrol admit gate (`initialize_patrol`):
            // `is_civilian() || is_able_to_fight()`.
            is_civilian: bool,
            is_able_to_fight: bool,
        }
        let mut snaps: std::collections::HashMap<EntityId, NpcSnap> =
            std::collections::HashMap::new();
        for &npc_id in &npc_ids {
            let Some(entity) = self.entities.get(npc_id) else {
                continue;
            };
            let pos = entity.element_data().position_map();
            let dir = entity.element_data().direction();
            let sector = entity.element_data().sector();
            let layer = entity.element_data().layer();
            let ai_state = entity
                .npc_data()
                .map(|n| n.ai_state())
                .unwrap_or(AiState::Sleeping);
            let view_radius = entity.npc_data().map(|n| n.view_radius).unwrap_or(400);
            let move_box = *entity.position_iface().get_move_box();
            let is_civilian = entity.is_civilian();
            let is_able_to_fight = match entity {
                crate::element::Entity::Soldier(s) => {
                    use crate::element::Human as _;
                    s.is_able_to_fight()
                }
                crate::element::Entity::Pc(pc) => {
                    use crate::element::Human as _;
                    pc.is_able_to_fight()
                }
                // Civilians, props, etc.: the default
                // `is_able_to_fight()` is `false` — but civilians flow
                // through the `is_civilian()` arm of the patrol gate
                // instead.
                _ => false,
            };

            snaps.insert(
                npc_id,
                NpcSnap {
                    position: Position {
                        x: pos.x,
                        y: pos.y,
                        sector,
                        level: layer,
                    },
                    direction: dir as u16,
                    ai_state,
                    is_alive: !entity.is_dead(),
                    is_active: entity.is_active(),
                    view_radius,
                    move_box,
                    is_civilian,
                    is_able_to_fight,
                },
            );
        }

        // ── Phase 3: Initialize patrols + compute formation positions ──
        struct PatrolCmd {
            minion: EntityId,
            target: Position,
            direction: u16,
        }
        let mut patrol_cmds: Vec<PatrolCmd> = Vec::new();
        let mut chief_assigns: Vec<(EntityId, EntityId)> = Vec::new(); // (minion, chief)

        for &npc_id in &npc_ids {
            // `refresh_patrol`: chiefs in {Flying, OnLadder, OnWall}
            // or mid-{PassDoor, Fall} sequence command skip the
            // entire tick — formation targets would trail an unusable
            // position and the 16-pixel side offset would still get
            // dispatched.  Check before acquiring the entity/ai borrow
            // so the engine-level helper can read `self`.
            if self.is_very_very_busy(npc_id) {
                continue;
            }
            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };
            if !entity.is_active() || entity.is_dead() {
                continue;
            }
            let Some(ai) = entity.ai_controller_mut() else {
                continue;
            };

            // Skip non-chiefs
            if ai.theoretical_patrol.is_empty() {
                continue;
            }

            // ── Initialize patrol on the one-shot reinit trigger ──
            // `initialize_patrol()` is called explicitly from
            // `init_one_ai`, `return_to_duty`, the `CMD_PATROL_START`
            // macro opcode, and the `Substate::DefaultGotoRoute`
            // EVENT_REACHPOINT handler — all of which set
            // `needs_patrol_reinit` on the chief.  Switching on the
            // flag (instead of "both lists empty") prevents a chief
            // whose minions all died/were promoted out from silently
            // re-initialising every tick — such chiefs stay in the
            // `patrol_size == 0 && missed == 0` early-return.  When
            // the flag fires we clear `patrol` and
            // `missed_patrol_members` before re-populating from
            // `theoretical_patrol`.
            if ai.needs_patrol_reinit {
                ai.needs_patrol_reinit = false;
                ai.patrol.clear();
                ai.missed_patrol_members.clear();
                let theoretical = ai.theoretical_patrol.clone();
                let chief_snap = snaps.get(&npc_id).copied();
                let chief_pos = chief_snap.map(|s| s.position).unwrap_or_default();
                let chief_view_radius = chief_snap.map(|s| s.view_radius as f32).unwrap_or(0.0);
                let chief_view_radius_sq = chief_view_radius * chief_view_radius;
                let obstacles_owned = scratch.ai_sight_obstacles.clone();
                let obstacles = obstacles_owned.list();

                for &member in &theoretical {
                    if member == npc_id {
                        continue;
                    }
                    if let Some(snap) = snaps.get(&member) {
                        // `initialize_patrol`: admit only if
                        // `is_detecting_360_degrees(member) &&
                        // ai_state == Default && (is_civilian() ||
                        // is_able_to_fight())`.  Members failing the
                        // gate but still alive flow into the missed
                        // list for later re-acquisition.
                        let mut admit = snap.is_active
                            && snap.is_alive
                            && snap.ai_state == AiState::Default
                            && (snap.is_civilian || snap.is_able_to_fight);
                        if admit {
                            // `is_detecting_360_degrees`: isometric
                            // squared distance vs view radius² + opaque
                            // LOS via `FastFindGrid::is_reachable`.
                            let dx = snap.position.x - chief_pos.x;
                            let dy = snap.position.y - chief_pos.y;
                            let sqr_dist =
                                crate::position_interface::vector_square_norm_iso(dx, dy);
                            if sqr_dist > chief_view_radius_sq {
                                admit = false;
                            } else {
                                let viewer =
                                    crate::coordinates::MapPoint::new(chief_pos.x, chief_pos.y);
                                let target = crate::coordinates::MapPoint::new(
                                    snap.position.x,
                                    snap.position.y,
                                );
                                if !crate::ai_vision::los_clear_spatial(
                                    viewer,
                                    target,
                                    chief_pos.level,
                                    obstacles,
                                    &self.fast_grid,
                                ) {
                                    admit = false;
                                }
                            }
                        }
                        if admit {
                            ai.patrol.push(member);
                            chief_assigns.push((member, npc_id));
                        } else if snap.is_alive {
                            ai.missed_patrol_members.push(member);
                        }
                    }
                }

                // Sort patrol members by distance to chief
                // (square-distance, stable sort).
                let snap_ref = &snaps;
                ai.patrol.sort_by(|a, b| {
                    let da = snap_ref.get(a).map_or(f32::MAX, |s| {
                        let dx = s.position.x - chief_pos.x;
                        let dy = s.position.y - chief_pos.y;
                        dx * dx + dy * dy
                    });
                    let db = snap_ref.get(b).map_or(f32::MAX, |s| {
                        let dx = s.position.x - chief_pos.x;
                        let dy = s.position.y - chief_pos.y;
                        dx * dx + dy * dy
                    });
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                });

                // Arrange left/right pairs: for each pair, ensure
                // even-index member is to the left of the odd-index
                // one (relative to chief).  Uses a 2D determinant.
                let patrol_size = ai.patrol.len();
                for i in (1..patrol_size).step_by(2) {
                    let even_h = ai.patrol[i - 1];
                    let odd_h = ai.patrol[i];
                    if let (Some(even_s), Some(odd_s)) =
                        (snap_ref.get(&even_h), snap_ref.get(&odd_h))
                    {
                        let ex = even_s.position.x - chief_pos.x;
                        let ey = even_s.position.y - chief_pos.y;
                        let ox = odd_s.position.x - chief_pos.x;
                        let oy = odd_s.position.y - chief_pos.y;
                        // 2D determinant: if even is on the wrong side, swap
                        if ex * oy - ey * ox < 0.0 {
                            ai.patrol.swap(i - 1, i);
                        }
                    }
                }
            }

            // ── Refresh patrol positions ──
            let patrol_size = ai.patrol.len();
            if patrol_size == 0 && ai.missed_patrol_members.is_empty() {
                continue;
            }
            if ai.patrol_stopped {
                continue;
            }
            if ai.current_state != AiState::Default {
                continue;
            }
            if ai.current_substate == Substate::DefaultPatrolChiefReturnToPatrol {
                continue;
            }

            // Must have a patrol path to track history
            let Some(ref mut path) = ai.patrol_path else {
                continue;
            };

            // Record history entry every frame
            if let Some(snap) = snaps.get(&npc_id) {
                path.add_history_entry(snap.position, snap.direction as u8);
            }

            // Every 8th frame: compute positions and coordinate minions
            if (frame & 7) != 0 {
                continue;
            }

            if patrol_size == 0 {
                // No active members but there may be missed ones — check below
            } else {
                // Expand the chief's move box by 3 on each side
                // before feeding it to
                // `is_straight_movement_autorized` for the 3-step
                // side-offset fallback.
                let chief_box = match snaps.get(&npc_id).map(|s| s.move_box) {
                    Some(b) if b.is_somewhere() => crate::coordinates::MoveBox::from_coords(
                        b.x_min() - 3.0,
                        b.y_min() - 3.0,
                        b.x_max() + 3.0,
                        b.y_max() + 3.0,
                    ),
                    _ => crate::coordinates::MoveBox::new(),
                };
                let positions =
                    path.compute_patrol_positions(patrol_size, Some(&self.fast_grid), &chief_box);
                let patrol_members = ai.patrol.clone();

                for (i, &member) in patrol_members.iter().enumerate() {
                    if let Some(&(ref pos, dir)) = positions.get(i) {
                        // Only coordinate if member is far enough from target (MaxNorm > 3)
                        if let Some(member_snap) = snaps.get(&member) {
                            let dx = (member_snap.position.x - pos.x).abs();
                            let dy = (member_snap.position.y - pos.y).abs();
                            if dx.max(dy) > 3.0 {
                                patrol_cmds.push(PatrolCmd {
                                    minion: member,
                                    target: *pos,
                                    direction: dir,
                                });
                            }
                        }
                    }
                }
            }

            // Check missed patrol members for re-acquisition.
            // `is_detecting_360_degrees`: isometric squared distance
            // check (Y stretched by INVERSE_ASPECT_RATIO) plus the
            // `FastFindGrid::is_reachable(OPAQUE)` LOS gate — a
            // separated minion behind a wall must NOT re-join even
            // within view radius.
            let chief_snap = snaps.get(&npc_id).copied();
            let missed = ai.missed_patrol_members.clone();
            let mut reacquired = Vec::new();
            let obstacles_owned = scratch.ai_sight_obstacles.clone();
            let obstacles = obstacles_owned.list();
            for (i, &member) in missed.iter().enumerate() {
                if let (Some(chief_s), Some(member_s)) = (chief_snap, snaps.get(&member))
                    && member_s.is_active
                    && member_s.is_alive
                    && member_s.ai_state == AiState::Default
                {
                    let dx = member_s.position.x - chief_s.position.x;
                    let dy = member_s.position.y - chief_s.position.y;
                    let sqr_dist = crate::position_interface::vector_square_norm_iso(dx, dy);
                    let radius = chief_s.view_radius as f32;
                    if sqr_dist > radius * radius {
                        continue;
                    }
                    let viewer =
                        crate::coordinates::MapPoint::new(chief_s.position.x, chief_s.position.y);
                    let target =
                        crate::coordinates::MapPoint::new(member_s.position.x, member_s.position.y);
                    if !crate::ai_vision::los_clear_spatial(
                        viewer,
                        target,
                        chief_s.position.level,
                        obstacles,
                        &self.fast_grid,
                    ) {
                        continue;
                    }
                    reacquired.push(i);
                    ai.patrol.push(member);
                    chief_assigns.push((member, npc_id));
                }
            }
            for &i in reacquired.iter().rev() {
                ai.missed_patrol_members.remove(i);
            }
        }

        // ── Phase 4: Set patrol_chief on minions ──
        for (minion, chief) in chief_assigns {
            if let Some(entity) = self.entities.get_mut(minion)
                && let Some(ai) = entity.ai_controller_mut()
            {
                ai.patrol_chief = Some(chief);
            }
        }

        // ── Phase 5: Build per-minion patrol tick data map ──
        // Build a map of minion → (chief_position, chief_state) for use
        // in the coordinate dispatch below.
        let mut patrol_tick_map: std::collections::HashMap<
            EntityId,
            (crate::ai::Position, crate::ai::AiState),
        > = std::collections::HashMap::new();
        for &npc_id in &npc_ids {
            let Some(entity) = self.entities.get(npc_id) else {
                continue;
            };
            let Some(ai) = entity.ai_controller() else {
                continue;
            };
            if let Some(chief_id) = ai.patrol_chief
                && let Some(cs) = snaps.get(&chief_id)
            {
                patrol_tick_map.insert(npc_id, (cs.position, cs.ai_state));
            }
        }

        // ── Phase 6: Dispatch CALL_PATROL_COORDINATE to minions ──
        let patrol_frame = self.frame_counter;
        for cmd in patrol_cmds {
            let minion_id = cmd.minion;
            let ctx = {
                let Some(entity) = self.entities.get_mut(minion_id) else {
                    continue;
                };
                let ctx = build_ai_context_from_entity(
                    entity,
                    patrol_frame,
                    None,
                    self.weather.is_forest_level,
                    self.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.fast_grid,
                    &assets.hiking_paths,
                    &self.ai_global.all_soldier_handles,
                );

                // Instruct facing direction (in-scope mut borrow).
                if let Some(ai) = entity.ai_controller_mut() {
                    ai.set_instructed_patrol_direction(cmd.direction, &ctx);
                }
                ctx
            };

            // Build tick data with patrol chief info.  Use the
            // centralized builder so combat-path fields stay
            // populated — patrol minions can be alerted mid-patrol
            // and dispatched into battle decisions without losing
            // their primary target snapshot.
            let mut tick_data = self.build_npc_tick_data(minion_id, &scratch, assets);
            if let Some(&(chief_pos, chief_state)) = patrol_tick_map.get(&cmd.minion) {
                tick_data.patrol_chief_position = chief_pos;
                tick_data.patrol_chief_state = chief_state;
            }

            // Dispatch CALL_PATROL_COORDINATE through the script filter.
            let stimulus = Stimulus::with_position(StimulusType::CallPatrolCoordinate, cmd.target);
            self.dispatch_filtered_stimulus(assets, minion_id, &stimulus, &ctx, &tick_data);
        }
    }

    // ─── One-shot noise broadcast ──────────────────────────────────

    /// Broadcast a one-shot noise event to all nearby NPCs.
    ///
    /// Called by projectile impacts, trap activations, scripted bridges,
    /// etc.  Filters to `is_civilian() || camp == Lacklandists` —
    /// royalist soldiers (player-controlled) do not receive broadcast
    /// noise stimuli.  Per-NPC subjective volume follows the
    /// `get_hear_volume` formula (volume×hearing_factor − iso-stretched
    /// distance − deafness).
    pub fn broadcast_noise(
        &mut self,
        noise_type: crate::ai::NoiseType,
        origin: crate::coordinates::MapPoint,
        origin_layer: u16,
        volume: u16,
        elevation: u16,
        source_entity: Option<EntityId>,
    ) {
        use crate::ai::{Noise, NoiseType, Position, Stimulus, StimulusType};

        let noise_pos = Position {
            x: origin.x,
            y: origin.y,
            sector: None,
            level: origin_layer,
        };

        // Only stamp the source's creation order on TAPTAPTAP /
        // ZINGZING / AAARGH / HEEELP — the four "attributable" cries;
        // other types (BONK, ZONK, PFIIIT, PLING, NOISE_LOGS,
        // NOISE_DRAWBRIDGE, PLOUF) leave the field unset.  EntityId
        // stands in for `creation_order`.
        let element_id = match noise_type {
            NoiseType::TapTapTap | NoiseType::ZingZing | NoiseType::Aaargh | NoiseType::Heeelp => {
                source_entity.map(|id| id.index() as u16).unwrap_or(0)
            }
            _ => 0,
        };

        // Queue the full-volume noise for the `noise_display` debug
        // overlay.  Host drains `SideEffects::displayed_noises` after
        // the tick, respecting the cheat flag.  The reference
        // `RHEngine::AddNoiseToDisplay` copies the full `RHnoise`, so
        // the displayed and per-NPC subjective copies both preserve
        // the attributable `element_id`.
        self.pending_side_effects.displayed_noises.push(Noise {
            origin: noise_pos,
            noise_type,
            volume,
            elevation,
            element_id,
        });

        // `hearing_factor` is a class-level static, default 1.0, with
        // no setter wired in shipped gameplay.  Apply the same
        // constant to every listener.
        const HEARING_FACTOR: f32 = 1.0;

        let frame = self.frame_counter;
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();

        // `get_hear_volume` shifts the origin Y by elevation and
        // keeps elevation as Z, so an elevated noise source
        // (drawbridge, arrow on a roof) is perceptually farther from
        // a ground-level listener.  Listener Z is read from
        // `elem.position().z` below and folded into the `dz` term.
        let elev_f = elevation as f32;

        for npc_id in npc_ids {
            // First pass: gather everything we need from the entity that
            // is independent of `self.sound_sim`.  Drop the borrow
            // before computing `cover_volume` so the
            // `&self.sound_sim` access below is non-overlapping.
            let (npc_pos, npc_elev) = {
                let Some(entity) = self.entities.get_mut(npc_id) else {
                    continue;
                };

                // Only civilians and Lacklandist soldiers listen.
                let include = match entity {
                    Entity::Civilian(_) => true,
                    Entity::Soldier(s) => {
                        s.soldier.cached_camp == crate::element_kinds::Camp::Lacklandists
                    }
                    _ => continue,
                };
                if !include {
                    continue;
                }

                let elem = entity.element_data();
                if !elem.active {
                    continue;
                }
                let unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);
                if unconscious {
                    continue;
                }
                (elem.position_map(), elem.position().z)
            };

            // `noise()` does NOT filter by layer; every in-camp NPC
            // is passed through `get_hear_volume`, which uses pure 3D
            // distance.

            // `get_hear_volume` formula.
            let modified_volume = volume as f32 * HEARING_FACTOR;
            let dx = npc_pos.x - origin.x;
            let dy_stretched =
                (npc_pos.y - origin.y - elev_f) * crate::position_interface::INVERSE_ASPECT_RATIO;
            // `distance = position - origin` with `origin.z =
            // elevation`, so dz = listener.z - source.elevation.
            let dz = npc_elev - elev_f;
            if dx.abs().max(dy_stretched.abs()).max(dz.abs()) > modified_volume {
                continue;
            }

            // Fold the max covering volume from active sound sources
            // at the NPC's position into the deafness write-back.
            let cover_volume = self
                .sound_sim
                .sources
                .max_noise_covering_volume_for_3d(npc_pos.x, npc_pos.y, npc_elev);

            // Re-borrow the entity for the deafness read + stimulus
            // push.  `noise()` has no state pre-filter: every in-camp
            // NPC in hearing range is passed to `think(stimulus)` and
            // the state machine decides.
            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };
            let deafness = {
                let Some(npc) = entity.npc_data_mut() else {
                    continue;
                };
                npc.get_deafness(frame, cover_volume) as f32
            };

            let distance = (dx * dx + dy_stretched * dy_stretched + dz * dz).sqrt();
            let subjective = modified_volume - distance - deafness;
            if subjective <= 0.0 {
                continue;
            }

            // Queue EventHear for the post-AI `pending_stimuli` drain so
            // `FilterAIEvent` can run with entities available (the
            // filter needs `swap_engine_state`, which conflicts with any
            // entity mut borrow we might hold here).
            let noise = Noise {
                origin: noise_pos,
                noise_type,
                volume: subjective as u16,
                elevation,
                element_id,
            };
            let stimulus = Stimulus::with_noise(StimulusType::EventHear, noise);
            if let Some(ai) = entity.ai_controller_mut() {
                ai.pending_stimuli.push(stimulus);
            }
        }
    }

    // ── Cross-NPC action processing (phalanx coordination) ──────────
    //
    // After all AI think() calls, drain each NPC's pending cross-NPC
    // actions and apply them to the target NPCs. This covers:
    // - InstructGatherPosition + CALL_INSTRUCTION delivery
    // - BreakPhalanx propagation
    // - SendStimulus (e.g. CALL_COORDINATE to archers)
    // - SetLeft/RightCombatNeighbour for phalanx linking

    pub(super) fn process_pending_cross_npc_actions(&mut self, assets: &LevelAssets) {
        let scratch = self.build_sim_scratch(assets);
        // Collect all pending actions first to avoid borrow issues.
        // Both enemy (soldier) and friendly (civilian) AIs can push
        // cross-NPC actions — e.g. civilians send `CALL_ALERT` /
        // `CALL_REPORT` to soldiers via `AiController` on their base.
        let mut all_actions: Vec<crate::ai::CrossNpcAction> = Vec::new();
        for (_, entity) in self.entities.npcs_mut() {
            if let Some(ai) = entity.ai_controller_mut() {
                all_actions.extend(ai.take_pending_cross_npc_actions());
            }
        }

        if all_actions.is_empty() {
            // No cross-NPC actions to process, but still deliver any
            // self-stimuli queued last tick (EventDone from
            // `SendCondolationCard`, MYTALK callbacks, etc.).  This
            // drain used to live at the tail of this function, which
            // meant it was skipped entirely on ticks with no cross-NPC
            // actions — the common case — stranding queued stimuli
            // forever and hanging states like
            // `DefaultOnPostLookingSidewards` that wait on `EventDone`
            // to exit.
            self.drain_pending_self_stimuli(assets);
            return;
        }

        let frame = self.frame_counter;

        for action in all_actions {
            match action {
                crate::ai::CrossNpcAction::InstructGatherPosition {
                    target,
                    position,
                    direction,
                } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let ctx = {
                        let Some(entity @ Entity::Soldier(_)) = self.entities.get_mut(target_id)
                        else {
                            continue;
                        };
                        let ctx = build_ai_context_from_entity(
                            entity,
                            frame,
                            None,
                            self.weather.is_forest_level,
                            self.standard_view_polygon_radius,
                            &scratch.ai_entity_views,
                            &scratch.ai_sight_obstacles,
                            &self.fast_grid,
                            &assets.hiking_paths,
                            &self.ai_global.all_soldier_handles,
                        );
                        let Entity::Soldier(s) = entity else {
                            unreachable!()
                        };
                        if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                            enemy_ai.gather_position = position;
                            enemy_ai.gather_direction = direction;
                            enemy_ai.gather_position_instructed = true;
                        }
                        ctx
                    };
                    // CrossNpcAction::InstructGatherPosition: target
                    // is an enemy soldier.  Build rich tick data so a
                    // subsequent think()-triggered BattleDecisions
                    // sees the target snapshot.
                    let tick_data = self.build_npc_tick_data(target_id, &scratch, assets);
                    let stimulus = crate::ai::Stimulus::new(StimulusType::CallInstruction);
                    self.dispatch_filtered_stimulus(assets, target_id, &stimulus, &ctx, &tick_data);
                }

                crate::ai::CrossNpcAction::BreakPhalanx { target } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let ctx = {
                        let Some(entity @ Entity::Soldier(_)) = self.entities.get_mut(target_id)
                        else {
                            continue;
                        };
                        let ctx = build_ai_context_from_entity(
                            entity,
                            frame,
                            None,
                            self.weather.is_forest_level,
                            self.standard_view_polygon_radius,
                            &scratch.ai_entity_views,
                            &scratch.ai_sight_obstacles,
                            &self.fast_grid,
                            &assets.hiking_paths,
                            &self.ai_global.all_soldier_handles,
                        );
                        let Entity::Soldier(s) = entity else {
                            unreachable!()
                        };
                        if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                            enemy_ai.left_combat_neighbour = 0;
                            enemy_ai.right_combat_neighbour = 0;
                            enemy_ai.phalanx_aborted = true;
                        }
                        ctx
                    };
                    // CrossNpcAction::BreakPhalanx: target is an
                    // enemy soldier breaking formation — ReturnToDuty
                    // may route through BattleDecisions.
                    let tick_data = self.build_npc_tick_data(target_id, &scratch, assets);
                    let stimulus = crate::ai::Stimulus::new(StimulusType::EventReturnToDuty);
                    self.dispatch_filtered_stimulus(assets, target_id, &stimulus, &ctx, &tick_data);
                }

                crate::ai::CrossNpcAction::SendStimulus {
                    target,
                    stimulus_type,
                    info,
                    fallback_to_sender,
                    to_whole_patrol,
                } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let mut stimulus = crate::ai::Stimulus::new(stimulus_type);
                    stimulus.info = info;
                    stimulus.to_whole_patrol = to_whole_patrol;

                    let ctx = {
                        let Some(entity @ Entity::Soldier(_)) = self.entities.get(target_id) else {
                            // Target missing → try fallback directly below.
                            if let Some(sender) = fallback_to_sender {
                                let sender_id = EntityId::Soldier(SoldierId(sender));
                                if let Some(entity @ Entity::Soldier(_)) =
                                    self.entities.get(sender_id)
                                {
                                    let ctx = build_ai_context_from_entity(
                                        entity,
                                        frame,
                                        None,
                                        self.weather.is_forest_level,
                                        self.standard_view_polygon_radius,
                                        &scratch.ai_entity_views,
                                        &scratch.ai_sight_obstacles,
                                        &self.fast_grid,
                                        &assets.hiking_paths,
                                        &self.ai_global.all_soldier_handles,
                                    );
                                    let fallback_tick =
                                        self.build_npc_tick_data(sender_id, &scratch, assets);
                                    self.dispatch_filtered_stimulus(
                                        assets,
                                        sender_id,
                                        &stimulus,
                                        &ctx,
                                        &fallback_tick,
                                    );
                                }
                            }
                            continue;
                        };
                        build_ai_context_from_entity(
                            entity,
                            frame,
                            None,
                            self.weather.is_forest_level,
                            self.standard_view_polygon_radius,
                            &scratch.ai_entity_views,
                            &scratch.ai_sight_obstacles,
                            &self.fast_grid,
                            &assets.hiking_paths,
                            &self.ai_global.all_soldier_handles,
                        )
                    };
                    // SendStimulus → enemy soldier target: the
                    // stimulus may be EVENT_VIEW / EVENT_REPORT /
                    // alert-forwarding which feeds BattleDecisions.
                    let tick_data = self.build_npc_tick_data(target_id, &scratch, assets);
                    let handled = self
                        .dispatch_filtered_stimulus(assets, target_id, &stimulus, &ctx, &tick_data);
                    // Fallback: if target couldn't handle the stimulus,
                    // redeliver to the sender (e.g. conversation chains).
                    if !handled && let Some(sender) = fallback_to_sender {
                        let sender_id = EntityId::Soldier(SoldierId(sender));
                        let ctx2 = {
                            let Some(entity @ Entity::Soldier(_)) = self.entities.get(sender_id)
                            else {
                                continue;
                            };
                            build_ai_context_from_entity(
                                entity,
                                frame,
                                None,
                                self.weather.is_forest_level,
                                self.standard_view_polygon_radius,
                                &scratch.ai_entity_views,
                                &scratch.ai_sight_obstacles,
                                &self.fast_grid,
                                &assets.hiking_paths,
                                &self.ai_global.all_soldier_handles,
                            )
                        };
                        let fallback_tick = self.build_npc_tick_data(sender_id, &scratch, assets);
                        self.dispatch_filtered_stimulus(
                            assets,
                            sender_id,
                            &stimulus,
                            &ctx2,
                            &fallback_tick,
                        );
                    }
                }

                crate::ai::CrossNpcAction::SetLeftCombatNeighbour { target, neighbour } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.left_combat_neighbour = neighbour;
                    }
                }

                crate::ai::CrossNpcAction::SetRightCombatNeighbour { target, neighbour } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.right_combat_neighbour = neighbour;
                    }
                }

                // Full reciprocal update.  Four steps:
                //   1. clear old_left's right pointer
                //   2. store new_left on target's left pointer (caller
                //      may also have written it eagerly for immediate
                //      visibility)
                //   3. pre-clean new_left's existing right (recursive
                //      `update_right_combat_neighbour(NULL)`) — clear
                //      that-right's left pointer
                //   4. wire new_left's right back to target
                crate::ai::CrossNpcAction::UpdateLeftCombatNeighbour {
                    target,
                    old_left,
                    new_left,
                } => {
                    // Step 1: old left's right pointer = 0.
                    if old_left != 0
                        && let Some(Entity::Soldier(s)) = self
                            .entities
                            .get_mut(EntityId::Soldier(SoldierId(old_left)))
                        && let Some(ai) = s.npc.ai_brain.enemy_mut()
                    {
                        ai.right_combat_neighbour = 0;
                    }
                    // Step 2: target.left = new_left.
                    if let Some(Entity::Soldier(s)) =
                        self.entities.get_mut(EntityId::Soldier(SoldierId(target)))
                        && let Some(ai) = s.npc.ai_brain.enemy_mut()
                    {
                        ai.left_combat_neighbour = new_left;
                    }
                    if new_left != 0 {
                        // Step 3: new_left's existing right's left = 0.
                        let new_lefts_old_right = self
                            .entities
                            .get(EntityId::Soldier(SoldierId(new_left)))
                            .and_then(|e| match e {
                                Entity::Soldier(s) => s.npc.ai_brain.enemy(),
                                _ => None,
                            })
                            .map(|ai| ai.right_combat_neighbour)
                            .unwrap_or(0);
                        if new_lefts_old_right != 0
                            && let Some(Entity::Soldier(s)) = self
                                .entities
                                .get_mut(EntityId::Soldier(SoldierId(new_lefts_old_right)))
                            && let Some(ai) = s.npc.ai_brain.enemy_mut()
                        {
                            ai.left_combat_neighbour = 0;
                        }
                        // Step 4: new_left.right = target.
                        if let Some(Entity::Soldier(s)) = self
                            .entities
                            .get_mut(EntityId::Soldier(SoldierId(new_left)))
                            && let Some(ai) = s.npc.ai_brain.enemy_mut()
                        {
                            ai.right_combat_neighbour = target;
                        }
                    }
                }

                // Same shape as `update_left_combat_neighbour`, for
                // the right side.
                crate::ai::CrossNpcAction::UpdateRightCombatNeighbour {
                    target,
                    old_right,
                    new_right,
                } => {
                    // Step 1: old right's left pointer = 0.
                    if old_right != 0
                        && let Some(Entity::Soldier(s)) = self
                            .entities
                            .get_mut(EntityId::Soldier(SoldierId(old_right)))
                        && let Some(ai) = s.npc.ai_brain.enemy_mut()
                    {
                        ai.left_combat_neighbour = 0;
                    }
                    // Step 2: target.right = new_right.
                    if let Some(Entity::Soldier(s)) =
                        self.entities.get_mut(EntityId::Soldier(SoldierId(target)))
                        && let Some(ai) = s.npc.ai_brain.enemy_mut()
                    {
                        ai.right_combat_neighbour = new_right;
                    }
                    if new_right != 0 {
                        // Step 3: new_right's existing left's right = 0.
                        let new_rights_old_left = self
                            .entities
                            .get(EntityId::Soldier(SoldierId(new_right)))
                            .and_then(|e| match e {
                                Entity::Soldier(s) => s.npc.ai_brain.enemy(),
                                _ => None,
                            })
                            .map(|ai| ai.left_combat_neighbour)
                            .unwrap_or(0);
                        if new_rights_old_left != 0
                            && let Some(Entity::Soldier(s)) = self
                                .entities
                                .get_mut(EntityId::Soldier(SoldierId(new_rights_old_left)))
                            && let Some(ai) = s.npc.ai_brain.enemy_mut()
                        {
                            ai.right_combat_neighbour = 0;
                        }
                        // Step 4: new_right.left = target.
                        if let Some(Entity::Soldier(s)) = self
                            .entities
                            .get_mut(EntityId::Soldier(SoldierId(new_right)))
                            && let Some(ai) = s.npc.ai_brain.enemy_mut()
                        {
                            ai.left_combat_neighbour = target;
                        }
                    }
                }

                crate::ai::CrossNpcAction::SetPrimaryTarget {
                    target,
                    primary_target,
                } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.base.primary_target = primary_target;
                    }
                }

                crate::ai::CrossNpcAction::Say { target, remark } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.base.say(remark);
                    }
                }

                crate::ai::CrossNpcAction::SetLootedAfterMoneyFight { target, looted } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.base.looted_after_money_fight = looted;
                    }
                }

                crate::ai::CrossNpcAction::UpdateReport {
                    target,
                    report_type,
                    seek_position,
                } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai
                            .base
                            .my_reconnaissance_report
                            .update(report_type, seek_position);
                    }
                }

                crate::ai::CrossNpcAction::ConsiderReport {
                    target,
                    report,
                    flags,
                } => {
                    let target_id = EntityId::Soldier(SoldierId(target));
                    let Some(Entity::Soldier(s)) = self.entities.get_mut(target_id) else {
                        continue;
                    };
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        // Use the AiController-level helper: it merges
                        // the report AND queues the per-body
                        // `delete_detectable(body, DETECTABLE_BODY)`
                        // side effects.  The bare
                        // `ReconnaissanceReport::consider_report`
                        // skipped those side effects, leaving stale
                        // body detectables on the NPC after a peer
                        // report merge.
                        enemy_ai.base.consider_report_merged(&report, flags);
                    }
                }

                crate::ai::CrossNpcAction::RegisterSynchronizingActor { target, actor } => {
                    // `register_synchronizing_actor` pushes the
                    // calling NPC onto the target's
                    // `synchronizing_actors` so the target's
                    // macro-complete dispatch can wake all waiters.
                    // Dedup the push for safety since the original
                    // list pushes unconditionally.
                    let target_id = EntityId::Soldier(SoldierId(target));
                    if let Some(entity) = self.entities.get_mut(target_id)
                        && let Some(ai) = entity.ai_controller_mut()
                        && !ai.synchronizing_actors.contains(&actor)
                    {
                        ai.synchronizing_actors.push(actor);
                    }
                }
            }
        }

        self.drain_pending_self_stimuli(assets);
    }

    /// Dispatch `stimulus` to `npc_id` via
    /// [`Self::dispatch_filtered_stimulus`], then run a synchronous
    /// side-effect drain pass so handler side effects (LaunchSequence,
    /// SetAttentiveMode, Face, quit/enter swordfight, look-sidewards,
    /// …) and any condolations / re-entrant `Think(EVENT_DONE)` they
    /// trigger happen inside the same call stack as the outer Think —
    /// matching the original `think()`, where handlers invoke
    /// `launch_sequence`, `halt`, `face`, `set_attentive_mode` inline
    /// and `send_condolation_card` fires `think(EVENT_DONE)`
    /// re-entrantly.
    ///
    /// The loop re-runs the drain while the NPC keeps generating new
    /// pending side effects (e.g. one condolation's `EventDone` handler
    /// queues another sequence that is preempted in the next iteration),
    /// bounded at 8 iterations to guard against a pathological cascade.
    ///
    /// Returns `dispatch_filtered_stimulus`'s handled bool — unchanged
    /// by the drain pass.
    pub(super) fn dispatch_think_with_drain(
        &mut self,
        npc_id: crate::element::EntityId,
        stimulus: &crate::ai::Stimulus,
        ctx: &crate::ai::AiContext,
        tick_data: &crate::ai::AiPerTickData,
        assets: &LevelAssets,
    ) -> bool {
        let handled = self.dispatch_filtered_stimulus(assets, npc_id, stimulus, ctx, tick_data);

        // Drain any panic-seek-point fallback the think pushed
        // (FleeingPanic / EventCouldntReachPoint arm).  Needs the
        // `ctx`/`ai_global` pair the outer caller owns, so it lives
        // here rather than in `drain_pending_for_npc`.
        self.process_pending_panic_seek_fallback_for(npc_id, ctx);

        const MAX_ITERS: u32 = 8;
        for iter in 0..MAX_ITERS {
            // Drain the per-NPC pending-flags pass (launches sequences,
            // commands, turn orders, attentive-mode transitions, etc.).
            self.drain_pending_for_npc(npc_id, assets);

            // Any condolations the drain above queued (sequences that
            // got preempted by the side effects) fire here — which may
            // push EventDone / EventImpossible into pending_self_stimuli.
            self.dispatch_condolations_for_npc(npc_id, assets);

            // Re-enter Think for each self-stimulus (EventDone, MYTALK,
            // etc.).  This may queue more pending flags — loop again.
            let had_self_stimuli = {
                let Some(entity) = self.entities.get(npc_id) else {
                    break;
                };
                let Some(ai) = entity.ai_controller() else {
                    break;
                };
                !ai.pending_self_stimuli.is_empty()
            };
            if !had_self_stimuli {
                break;
            }
            self.drain_self_stimuli_for_npc(npc_id, assets);

            if iter + 1 == MAX_ITERS {
                tracing::warn!(
                    npc = npc_id.index(),
                    "dispatch_think_with_drain reached MAX_ITERS without stabilising"
                );
            }
        }

        handled
    }

    /// Drain each NPC's `pending_self_stimuli` queue and re-dispatch each
    /// stimulus through `think` on the same frame.  Matches
    /// `Think()`-from-within-handler calls (MYTALK callbacks from
    /// `say()`, deferred `EventDone` from `SendCondolationCard`, etc.)
    /// which in the original engine immediately re-enter the AI but in
    /// Rust are queued to avoid nested `&mut AiGlobalState` borrows.
    ///
    /// Called unconditionally each tick.  Each NPC is drained to a fixed
    /// point so a Think call that recursively fires another self-stimulus
    /// observes that stimulus in the originating frame, matching the
    /// original direct `Think(...)` call.
    pub(super) fn drain_pending_self_stimuli(&mut self, assets: &LevelAssets) {
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for npc_id in npc_ids {
            self.drain_self_stimuli_for_npc(npc_id, assets);
        }
    }

    /// Per-NPC half of [`Self::drain_pending_self_stimuli`] — drains the
    /// pending self-stimulus queue for a single NPC and re-dispatches
    /// each through `think`.  Called both from the global end-of-tick
    /// drain and from [`Self::dispatch_think_with_drain`] so the
    /// re-entrant `think(EVENT_DONE)` that `send_condolation_card`
    /// fires lands inside the same call stack as the outer think.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    pub(super) fn drain_self_stimuli_for_npc(
        &mut self,
        npc_id: crate::element::EntityId,
        assets: &LevelAssets,
    ) {
        const MAX_REENTRANT_STIMULI: usize = 111;
        let mut dispatched = 0usize;

        loop {
            let stimulus_type = {
                let Some(entity) = self.entities.get_mut(npc_id) else {
                    return;
                };
                let Some(ai) = entity.ai_controller_mut() else {
                    return;
                };
                if ai.pending_self_stimuli.is_empty() {
                    break;
                }
                ai.pending_self_stimuli.remove(0)
            };

            dispatched += 1;
            if dispatched > MAX_REENTRANT_STIMULI {
                tracing::warn!(
                    npc = npc_id.index(),
                    "self-stimulus recursion exceeded the original 111-call guard"
                );
                break;
            }

            let scratch = self.build_sim_scratch(assets);
            let frame = self.frame_counter;
            let in_uninterruptible_command = self.is_very_very_busy(npc_id);
            let ctx = {
                let Some(entity) = self.entities.get(npc_id) else {
                    return;
                };
                let mut ctx = build_ai_context_from_entity(
                    entity,
                    frame,
                    None,
                    self.weather.is_forest_level,
                    self.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.fast_grid,
                    &assets.hiking_paths,
                    &self.ai_global.all_soldier_handles,
                );
                ctx.in_uninterruptible_command = in_uninterruptible_command;
                ctx
            };
            let stimulus = crate::ai::Stimulus::new(stimulus_type);
            // Pending self-stimuli drain: NPC could be enemy soldier
            // (most common — EventDone from SendCondolationCard,
            // MYTALK callbacks) or civilian.  Builder stubs for
            // non-enemy, populates for enemy.
            let tick_data = self.build_npc_tick_data(npc_id, &scratch, assets);
            self.dispatch_filtered_stimulus(assets, npc_id, &stimulus, &ctx, &tick_data);
            // The re-entered think might have queued a panic-seek
            // fallback (FleeingPanic / EventCouldntReachPoint).
            self.process_pending_panic_seek_fallback_for(npc_id, &ctx);

            // Original Think calls execute their engine-facing side effects
            // before returning.  Close that window after every recursive
            // stimulus so a newly launched sequence participates in
            // arbitration before the next sibling stimulus is delivered.
            self.drain_pending_for_npc(npc_id, assets);
            self.dispatch_condolations_for_npc(npc_id, assets);
        }
    }

    // ── Per-waypoint ReachPoint dispatch ──────────────────────────
    //
    // Drain `pending_waypoint_script_reach_point` on every NPC:
    // dispatch `ReachPoint(actor)` on the waypoint's bound VM, then
    // synchronously re-enter `think(EventAfterScriptGoOn)` unless the
    // script transitioned the NPC into `DefaultScriptDriven`.  Runs
    // `execute_waypoint_script`, including the `script_enabled` gate
    // and the recursive `think()` call.  If no script is bound for
    // the waypoint (class missing), the recursive `think` still fires
    // — the "script was a no-op" branch when the bound class doesn't
    // transition state.
    pub(super) fn dispatch_pending_waypoint_scripts(&mut self, assets: &LevelAssets) {
        // `script_enabled` gate — when scripts are disabled, drain
        // the pending requests but skip both the VM dispatch and the
        // follow-up `EventAfterScriptGoOn` Think.
        let scripts_enabled = crate::engine::GlobalOptions::global()
            .as_ref()
            .map(|o| o.script_enabled)
            .unwrap_or(true);

        // Collect requests so we can release the entity borrow before
        // swapping engine state into the script host.  Always take the
        // `Option` so we don't leave stale state when scripts are off.
        let mut requests: Vec<(crate::element::EntityId, crate::ai::PathId, u8)> = Vec::new();
        for (npc_id, entity) in self.entities.npcs_mut() {
            let Some(ai) = entity.ai_controller_mut() else {
                continue;
            };
            if let Some((path_idx, wp_idx)) = ai.pending_waypoint_script_reach_point.take()
                && scripts_enabled
            {
                requests.push((npc_id.into(), path_idx, wp_idx));
            }
        }

        if requests.is_empty() {
            return;
        }

        // ── Phase 1: dispatch ReachPoint(actor) on every pending VM ──
        // Pattern mirrors the target/scroll callback sites (see
        // `listenable_calls` in the target block): one swap pair for
        // the whole batch.
        self.refresh_game_host_entity_state();
        if let Some(ref mut script) = self.mission_script {
            script.swap_engine_state(
                &mut self.entities,
                &mut self.ai_global,
                &mut self.fast_grid,
                &mut self.campaign,
                &mut self.mission_stat,
            );
            for &(npc_id, path_idx, wp_idx) in &requests {
                let actor_handle = crate::natives::GameHost::actor_handle(npc_id);
                match script.call_waypoint_function(path_idx, wp_idx, "ReachPoint", &[actor_handle])
                {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(
                            "Waypoint ReachPoint (path {path_idx}, wp {wp_idx}, actor {actor_handle}): {e}"
                        );
                        // Debug assert — `ReachPoint` is part of the
                        // `IWaypointScript` contract, so a bound
                        // instance failing the call is a bug.
                        debug_assert!(
                            false,
                            "Waypoint ReachPoint (path {path_idx}, wp {wp_idx}, actor {actor_handle}): {e}"
                        );
                    }
                }
            }
            script.swap_engine_state(
                &mut self.entities,
                &mut self.ai_global,
                &mut self.fast_grid,
                &mut self.campaign,
                &mut self.mission_stat,
            );
        }
        self.sync_game_host_post_script(assets);

        // ── Phase 2: synchronous Think(EventAfterScriptGoOn) ──
        // `think(EVENT_AFTER_SCRIPT_GO_ON)` fires immediately after
        // `reach_point` on the same call stack.  Replicate that by
        // calling `think()` directly here — not via
        // `pending_self_stimuli` (which would let cross-NPC actions
        // interleave) — unless the script pulled the NPC into
        // `DefaultScriptDriven`.
        //
        // Scripts may have spawned / deactivated entities, so refresh
        // the entity-views map before rebuilding per-NPC AiContexts.
        let scratch = self.build_sim_scratch(assets);
        let frame = self.frame_counter;
        let is_forest_level = self.weather.is_forest_level;
        let standard_view_polygon_radius = self.standard_view_polygon_radius;
        for (npc_id, _, _) in requests {
            let ctx = {
                let Some(entity) = self.entities.get(npc_id) else {
                    continue;
                };
                // Read substate without holding a mutable borrow.
                let substate = entity
                    .ai_controller()
                    .map(|ai| ai.current_substate)
                    .unwrap_or(crate::ai::Substate::DefaultScriptDriven);
                if substate == crate::ai::Substate::DefaultScriptDriven {
                    continue;
                }
                build_ai_context_from_entity(
                    entity,
                    frame,
                    None,
                    is_forest_level,
                    standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.fast_grid,
                    &assets.hiking_paths,
                    &self.ai_global.all_soldier_handles,
                )
            };
            let stimulus = crate::ai::Stimulus::new(crate::ai::StimulusType::EventAfterScriptGoOn);
            // EventAfterScriptGoOn may re-enter BattleDecisions via
            // ThinkExpectedEventCommonStuff when the AI is attacking.
            let tick_data = self.build_npc_tick_data(npc_id, &scratch, assets);
            self.dispatch_think_with_drain(npc_id, &stimulus, &ctx, &tick_data, assets);
        }
    }

    // ── The16thFrame — periodic AI tasks (staggered) ──────────────
    //
    // `the_16th_frame` runs every 16th frame from the NPC's
    // `hourglass`, staggered by NPC index so not all soldiers run on
    // the same frame.

    pub(super) fn tick_periodic_ai(&mut self, assets: &LevelAssets) {
        if self.actors_frozen() || self.ai_global.freeze {
            return;
        }
        let scratch = self.build_sim_scratch(assets);

        let current_frame = self.frame_counter;

        for npc_id in self.entities.npc_ids().collect::<Vec<_>>() {
            // Exact original phase:
            //   (frame & 255) - ((register_number + 100) & 255)
            // with unsigned-byte wrap. Passing the full phase matters:
            // The16thFrame uses bits 4..5 to reduce some work to every
            // 64th frame, so substituting `frame % 16` ran that work 4x.
            let frame_phase = npc_hourglass_frame_phase(current_frame, npc_id.index());
            if (frame_phase & 15) != 0 {
                continue;
            }

            let locked = self.entities.get(npc_id).is_some_and(|entity| {
                entity
                    .ai_controller()
                    .is_some_and(|ai| !ai.locks_flag_field.is_empty() || ai.script_locked)
            });
            if locked {
                continue;
            }

            // `sequence_element_is_about_to_be_launched(self, NULL)`
            // — used by the civilian stuck-counter suppression.
            // Query once up front so we can hand it to the AI layer
            // without holding a sequence-manager borrow across the
            // AI tick.
            let sequence_null_about_to_launch = self
                .sequence_manager
                .element_is_about_to_be_launched(npc_id, crate::element::Command::Null);

            // `command == Wait` — entity is idle.  Read the live
            // sequence-element command via `actor_command` rather
            // than `action_state == Waiting` so we don't get a
            // false-positive on `WaitTimer` (which sets `action_state
            // = Waiting` via the animation map but is not
            // `Command::Wait`) or a false-negative on the brief
            // window where a teardown nulls the sequence-element
            // before the next animation tick resets `action_state`.
            let is_idle = self.actor_command(npc_id) == crate::element::Command::Wait;

            // Build the per-tick data first (uses `&self`) before the
            // mutable entity borrow.
            let tick_data = self.build_npc_tick_data(npc_id, &scratch, assets);

            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };

            if !entity.element_data().active || entity.is_dead() {
                continue;
            }

            let ctx = build_ai_context_from_entity(
                entity,
                current_frame,
                None,
                self.weather.is_forest_level,
                self.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.fast_grid,
                &assets.hiking_paths,
                &self.ai_global.all_soldier_handles,
            );

            match entity {
                Entity::Soldier(s) => {
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.the_16th_frame(
                            frame_phase,
                            &ctx,
                            &self.ai_global,
                            &tick_data,
                            Some(&self.fast_grid),
                            is_idle,
                            sequence_null_about_to_launch,
                        );
                    }
                }
                Entity::Civilian(c) => {
                    if let Some(friendly_ai) = c.npc.ai_brain.friendly_mut() {
                        friendly_ai.the_16th_frame(
                            frame_phase,
                            &mut self.ai_global,
                            is_idle,
                            sequence_null_about_to_launch,
                        );
                    }
                    // `tick_data` is only used for enemies; civilians
                    // don't need it.
                    let _ = &tick_data;
                }
                _ => {}
            }
        }
    }

    /// Civilian `RandomSpeech(ubFramePhase)` call from NPC Hourglass.
    /// It sits before the lock gate and only acts at exact phase zero.
    pub(super) fn tick_civilian_random_speech(&mut self, assets: &LevelAssets) {
        let current_frame = self.frame_counter;
        let due: Vec<_> = self
            .entities
            .npc_ids()
            .filter(|&npc_id| {
                matches!(self.entities.get(npc_id), Some(Entity::Civilian(_)))
                    && npc_hourglass_frame_phase(current_frame, npc_id.index()) == 0
            })
            .collect();
        if due.is_empty() {
            return;
        }

        let scratch = self.build_sim_scratch(assets);
        for npc_id in due {
            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };
            let ctx = build_ai_context_from_entity(
                entity,
                current_frame,
                None,
                self.weather.is_forest_level,
                self.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.fast_grid,
                &assets.hiking_paths,
                &self.ai_global.all_soldier_handles,
            );
            if let Some(friendly_ai) = entity.friendly_ai_mut() {
                friendly_ai.random_speech(0, &ctx);
            }
        }
    }

    // ── RefreshAmbushPoints — per-frame ambush peek scan ─────────
    //
    // `refresh_ambush_points` runs every frame for each NPC from
    // `hourglass`.  Civilians have a no-op virtual stub, so this only
    // fires for enemies (soldiers).  The per-NPC method updates the
    // slot status vector and may transition the AI substate via
    // `check_ambush_point`.

    pub(super) fn tick_refresh_ambush_points(&mut self, assets: &LevelAssets) {
        if self.actors_frozen() || self.ai_global.freeze {
            return;
        }
        if self.ai_global.ambush_points.is_empty() {
            return;
        }
        let scratch = self.build_sim_scratch(assets);

        let frame = self.frame_counter;
        let is_forest_level = self.weather.is_forest_level;
        let standard_view_polygon_radius = self.standard_view_polygon_radius;
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();

        for npc_id in npc_ids {
            // Phase 1: read-only — gather context + eyes point + LOS scope.
            let (ctx, eyes) = {
                let Some(entity) = self.entities.get(npc_id) else {
                    continue;
                };
                if !entity.element_data().active {
                    continue;
                }
                // Soldier-only: civilian RefreshAmbushPoints is a no-op.
                if entity.enemy_ai().is_none() {
                    continue;
                }
                let Some(eyes) = entity.compute_eyes_point(None) else {
                    continue;
                };
                let ctx = build_ai_context_from_entity(
                    entity,
                    frame,
                    None,
                    is_forest_level,
                    standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.fast_grid,
                    &assets.hiking_paths,
                    &self.ai_global.all_soldier_handles,
                );
                (ctx, eyes)
            };

            // Build the obstacle view from individual disjoint fields
            // so the borrow checker can split it from the mut borrow
            // on `self.entities` below.
            let sight_obstacles = crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.static_sight_obstacles.as_slice(),
                dynamic_obstacles: &self.dynamic_sight_obstacles,
                static_active: &self.static_sight_obstacle_active,
            };
            let ambush_points = self.ai_global.ambush_points.as_slice();

            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };
            if let Some(enemy_ai) = entity.enemy_ai_mut() {
                enemy_ai.refresh_ambush_points(&ctx, eyes, ambush_points, sight_obstacles);
            }
        }
    }

    // ── Macro timer hourglass ────────────────────────────────────
    //
    // `hourglass` polls `macro_timer_is_running` each frame and, when
    // the timer has rung and the NPC is still in
    // `SUBSTATE_DEFAULT_INMACRO`, calls `execute_next_macro_command()`
    // directly — **bypassing** the Think stimulus dispatch so
    // CMD_WAIT / CMD_BEND resume without going through EVENT_TIMER.
    //
    // We iterate both soldier and civilian NPCs because civilians use
    // the common macro opcodes too (REVERSE_PATH, WAIT, GOTO_POINT,
    // FACE_TO, ...).
    pub(super) fn tick_ai_macro_timers(&mut self, assets: &LevelAssets) {
        if self.actors_frozen() || self.ai_global.freeze {
            return;
        }
        let scratch = self.build_sim_scratch(assets);

        let current_frame = self.frame_counter;

        // Snapshot the list of NPC ids we'll iterate.  `npc_ids`
        // already holds both soldiers and civilians
        // (engine/mod.rs:1081-1096) — both kinds execute waypoint
        // macros, even if the soldier-only opcodes SBError on
        // civilians.
        let npc_ids: Vec<EntityId> = self.entities.npc_ids().collect();

        for npc_id in npc_ids {
            // Read macro-timer state without holding a borrow. The original
            // stops an elapsed macro timer even if the NPC has since left
            // DefaultInMacro; only command execution is substate-gated.
            let (fire, execute) = {
                let Some(entity) = self.entities.get(npc_id) else {
                    continue;
                };
                let base = match entity {
                    Entity::Soldier(s) => s.npc.ai_brain.base(),
                    Entity::Civilian(c) => c.npc.ai_brain.base(),
                    _ => None,
                };
                base.map(|ai| {
                    let unlocked = ai.locks_flag_field.is_empty() && !ai.script_locked;
                    let fire = unlocked
                        && ai.macro_timer_is_running
                        && ai.when_does_macro_timer_ring <= current_frame;
                    (
                        fire,
                        fire && ai.current_substate == crate::ai::Substate::DefaultInMacro,
                    )
                })
                .unwrap_or((false, false))
            };
            if !fire {
                continue;
            }

            // Build the AI context before we take the mut AI borrow.
            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };
            let ctx = build_ai_context_from_entity(
                entity,
                current_frame,
                None,
                self.weather.is_forest_level,
                self.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.fast_grid,
                &assets.hiking_paths,
                &self.ai_global.all_soldier_handles,
            );

            // Stop the timer and resume the macro VM.  `execute_next_
            // macro_command` may transition the substate (e.g. to
            // `DefaultEnroute` when the byte stream ends) — we don't
            // post-process beyond that; any downstream state changes
            // ride the normal think dispatch.
            let base_opt = match entity {
                Entity::Soldier(s) => s.npc.ai_brain.base_mut(),
                Entity::Civilian(c) => c.npc.ai_brain.base_mut(),
                _ => None,
            };
            if let Some(base) = base_opt {
                base.macro_timer_is_running = false;
                if execute {
                    base.execute_next_macro_command(&ctx);
                }
            }
        }
    }

    // ── Locked-frame timer bumps ─────────────────────────────────
    //
    // `hourglass` short-circuits the post-Refresh tail when any lock
    // is held (`locks_flag_field > 0 || script_locked || frozen_all`)
    // but still bumps `when_does_timer_ring`,
    // `when_does_macro_timer_ring`, and `emoticon_expiration_date`
    // per locked frame.  Without this, the per-piece tick guards
    // skip everything (no bumps), so ring-times shift -N once the
    // lock clears — a script-locked civilian's EVENT_TIMER would
    // fire immediately on unlock instead of N frames later.
    //
    // Bumping here also doubles as the "skip the fire" gate for the
    // downstream tick functions: `tick_ai_macro_timers` and the EVENT_
    // TIMER dispatch in `tick_enemy_ai_pursuit_approach` both check
    // `when_does_*_timer_ring <= current_frame` to decide whether to
    // fire.  Bumping the ring frame in lock-step with the lock keeps
    // it strictly greater than `current_frame`, preventing a fire.
    pub(super) fn tick_npc_locked_frame_timer_bumps(&mut self) {
        let frozen = self.actors_frozen();
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for npc_id in npc_ids {
            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                continue;
            };
            let locked = frozen || !ai.locks_flag_field.is_empty() || ai.script_locked;
            if !locked {
                continue;
            }
            ai.when_does_timer_ring = ai.when_does_timer_ring.saturating_add(1);
            ai.when_does_macro_timer_ring = ai.when_does_macro_timer_ring.saturating_add(1);
            ai.emoticon_expiration_date = ai.emoticon_expiration_date.saturating_add(1);
        }
    }

    // ── Stuck-on-ladder emergency counter ────────────────────────
    //
    // `hourglass` bumps `stuck_on_ladder_emergency_counter` every
    // frame an NPC is on a ladder in a non-building sector with
    // command `Wait`/`MoveWaiting` and not script-locked; otherwise
    // resets to 0.  After 25 frames it calls `force_return_to_duty()`
    // (== `return_to_duty()`) and resets the counter so
    // outdoor-ladder hangs self-recover.
    //
    // Note: this checks only `script_locked`, *not* `locks_flag_field`
    // — so the freshly-set BUSY lock from the edge detector earlier in
    // the same frame does not suppress this counter (the BUSY lock is
    // exactly what we want to escape from).
    pub(super) fn tick_npc_stuck_on_ladder(&mut self, assets: &LevelAssets) {
        if self.actors_frozen() {
            return;
        }
        let scratch = self.build_sim_scratch(assets);
        let npc_ids: Vec<_> = self.entities.npc_ids().collect();
        for npc_id in npc_ids {
            // Snapshot the gating predicates without holding a borrow.
            let on_ladder = match self.entities.get(npc_id) {
                Some(entity) => entity.element_data().posture == crate::element::Posture::OnLadder,
                _ => false,
            };
            let cmd = self.actor_command(npc_id);
            let in_wait_or_move_waiting = matches!(
                cmd,
                crate::element::Command::Wait | crate::element::Command::MoveWaiting
            );
            let (script_locked, in_building) = match self.entities.get(npc_id) {
                Some(entity) => (
                    entity.ai_controller().is_some_and(|ai| ai.script_locked),
                    self.entity_data_inside_building(entity.element_data()),
                ),
                _ => (false, false),
            };
            let qualifies = on_ladder && in_wait_or_move_waiting && !script_locked && !in_building;

            // Bump or reset the counter; remember whether to fire.
            let trigger = {
                let Some(entity) = self.entities.get_mut(npc_id) else {
                    continue;
                };
                let Some(npc) = entity.npc_data_mut() else {
                    continue;
                };
                if qualifies {
                    npc.stuck_on_ladder_emergency_counter =
                        npc.stuck_on_ladder_emergency_counter.saturating_add(1);
                    if npc.stuck_on_ladder_emergency_counter > 25 {
                        npc.stuck_on_ladder_emergency_counter = 0;
                        true
                    } else {
                        false
                    }
                } else {
                    npc.stuck_on_ladder_emergency_counter = 0;
                    false
                }
            };
            if !trigger {
                continue;
            }

            // `force_return_to_duty == return_to_duty`.  Dispatch via
            // the AI subclass to mirror the virtual call.  Build the
            // ctx + tick data the way `tick_periodic_ai` does.
            let tick_data = self.build_npc_tick_data(npc_id, &scratch, assets);
            let frame = self.frame_counter;
            let in_uninterruptible_command = self.is_very_very_busy(npc_id);
            let Some(entity) = self.entities.get_mut(npc_id) else {
                continue;
            };
            let mut ctx = build_ai_context_from_entity(
                entity,
                frame,
                None,
                self.weather.is_forest_level,
                self.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.fast_grid,
                &assets.hiking_paths,
                &self.ai_global.all_soldier_handles,
            );
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            match entity {
                Entity::Soldier(s) => {
                    if let Some(enemy_ai) = s.npc.ai_brain.enemy_mut() {
                        enemy_ai.return_to_duty(crate::ai::DutyFlags::empty(), &ctx, &tick_data);
                    }
                }
                Entity::Civilian(c) => {
                    if let Some(friendly_ai) = c.npc.ai_brain.friendly_mut() {
                        friendly_ai.return_to_duty(crate::ai::DutyFlags::empty(), &ctx);
                    }
                }
                _ => {}
            }
        }
    }
}
