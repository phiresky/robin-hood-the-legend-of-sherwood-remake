//! Per-NPC visibility passes for `tick_enemy_ai`: blip detection (P2a),
//! enemy → PC `RefreshDetection` (P3), and royalist → enemy detection
//! (P3b).  All three operate on the snapshots built in [`super::snapshots`]
//! and dispatch or queue the resulting stimuli at the matching original
//! phase boundary.

use super::snapshots::{AiWorldView, Detection, HumanTarget, ObjectTarget};
use super::*;
use crate::ai::AiPerTickData;
use crate::ai_vision;
use crate::coordinates::MapPoint;
use crate::element::{Camp, Detectable, DetectableType, Entity, EntityId, Posture};

/// Royalist-detection scratch type: snapshot of one Lacklandist NPC as a
/// detection target for the per-royalist visibility pass (P3b).  Built
/// once at the top of [`EngineInner::tick_enemy_ai_royalist_detection`]
/// and threaded into the per-NPC body so the inner loop can iterate
/// without re-borrowing `self.world.entities`.
#[derive(Clone)]
struct NpcTarget {
    id: EntityId,
    position: MapPoint,
    layer: u16,
    posture: crate::element::Posture,
    action_state: crate::element::ActionState,
    building_sector: Option<crate::position_interface::SectorHandle>,
    eye_z: f32,
    /// 16-sector facing.  Only used for `LeaningOut`: the detection
    /// point projects `direction × 40` forward.
    direction: i16,
    /// Whether the target is currently passing through a door — used
    /// by the same-building visibility short-circuit.
    passing_door: bool,
    /// The projection obstacle this NPC target is currently standing
    /// on.  Used by the per-target `compute_view_radius` re-call.
    obstacle_idx: Option<crate::position_interface::ObstacleHandle>,
}

fn human_eye_point_for_visibility(entity: &Entity) -> (MapPoint, f32) {
    let Some(eye) = entity.compute_eyes_point(None) else {
        let position = entity.element_data().position();
        let position_map = entity.element_data().position_map();
        return (position_map, position.z);
    };
    let ground_z = entity.element_data().position().z;
    // `compute_eyes_point` returns the engine's render-space 3D point,
    // where `y = map_y + elevation`.
    // TODO(coord-parity): original C++ `ComputeVisibility` subtracts
    // `SBGeoPoint3D` eye/detection points directly. Rust visibility
    // currently consumes projected `MapPoint`s; audit before changing
    // that behavior.
    (MapPoint::from_world_xyz(eye.x, eye.y, ground_z), eye.z)
}

struct SoldierSightContext {
    eye: MapPoint,
    eye_z: f32,
    dir: i16,
    layer: u16,
    view_radius: u16,
    eye_status: crate::element::EyeStatus,
    current_state: crate::ai::AiState,
    current_substate: crate::ai::Substate,
    ai_locked: bool,
    view_forward: (f32, f32),
    real_half_aperture: f32,
    posture: crate::element::Posture,
    action_state: crate::element::ActionState,
    sector: Option<crate::position_interface::SectorHandle>,
    alert_status: crate::ai::AlertLevel,
    blipped: bool,
    position_map: MapPoint,
    camp: Camp,
    is_rider: bool,
    ignore_bodies: bool,
}

impl SoldierSightContext {
    fn from_viewer(entity: &Entity, required_camp: Camp) -> Option<Self> {
        let Entity::Soldier(soldier) = entity else {
            return None;
        };
        if !entity.is_active()
            || entity.is_dead()
            || soldier.soldier.cached_camp != required_camp
            || soldier.human.unconscious
            || soldier.element.posture == crate::element::Posture::Tied
        {
            return None;
        }

        // Original: `RHElementActorSoldier` owns
        // `RHArtificialMalignity` and calls its state directly from
        // Hourglass. An eligible live soldier without EnemyAi is invalid.
        let ai = soldier.npc.ai_brain.enemy().unwrap_or_else(|| {
            panic!("eligible active soldier has no EnemyAi brain during detection")
        });
        let current_substate = ai.base.current_substate;
        let ignore_bodies = matches!(
            current_substate,
            crate::ai::Substate::SeekingOfficerWaitForAlertingSoldier
                | crate::ai::Substate::SeekingOfficerGetAlertingReportFromSoldier
        );
        let view_direction = soldier.npc.view_direction;
        let position_map = soldier.element.position_map();
        let (eye, eye_z) = human_eye_point_for_visibility(entity);

        Some(Self {
            eye,
            eye_z,
            dir: soldier.element.direction(),
            layer: soldier.element.layer(),
            view_radius: soldier.npc.view_radius,
            eye_status: soldier.npc.eye_status,
            current_state: soldier.npc.ai_state(),
            current_substate,
            ai_locked: ai.base.ai_is_locked(),
            view_forward: (view_direction[0], view_direction[1]),
            real_half_aperture: soldier.npc.real_half_aperture,
            posture: soldier.element.posture,
            action_state: soldier.actor.action_state,
            sector: soldier.element.sector(),
            alert_status: ai.base.current_music_alert_status,
            blipped: soldier.element.blipped,
            position_map,
            camp: soldier.soldier.cached_camp,
            is_rider: soldier.soldier.rider,
            ignore_bodies,
        })
    }
}

fn attacking_reactiontime_enemy_near_enabled(
    combat_trainer: bool,
    substate: crate::ai::Substate,
    frame: u32,
    frame_when_enemy_detected: u32,
) -> bool {
    use crate::ai::Substate;

    if combat_trainer {
        return false;
    }
    match substate {
        Substate::AttackingReactiontimeTurning | Substate::AttackingReactiontime => true,
        Substate::AttackingApproachToObserve | Substate::AttackingObserve => {
            frame.wrapping_sub(frame_when_enemy_detected) < 100
        }
        _ => false,
    }
}

fn enemy_is_in_react_immediately_zone(
    origin: MapPoint,
    target: crate::ai::Position,
    posture: crate::element::Posture,
) -> bool {
    posture.triggers_enemy_near()
        && (target.x - origin.x).abs() <= 50.0
        && (target.y - origin.y).abs() <= 30.0
}

fn enemies_near_from_them_list(
    origin: MapPoint,
    list_them: &[u32],
    mut target_snapshot: impl FnMut(u32) -> Option<(crate::ai::Position, crate::element::Posture)>,
) -> Vec<u32> {
    list_them
        .iter()
        .copied()
        .filter(|&target| {
            target_snapshot(target).is_some_and(|(position, posture)| {
                enemy_is_in_react_immediately_zone(origin, position, posture)
            })
        })
        .collect()
}

fn queued_human_detection_stimuli(
    event_type: crate::ai::StimulusType,
    shadow_dispatches: Vec<crate::ai::Position>,
    rising_dispatches: Vec<EntityId>,
) -> Vec<crate::ai::Stimulus> {
    let mut stimuli = Vec::with_capacity(shadow_dispatches.len() + rising_dispatches.len());
    stimuli.extend(shadow_dispatches.into_iter().map(|position| {
        crate::ai::Stimulus::with_position(crate::ai::StimulusType::EventSeesShadow, position)
    }));
    stimuli.extend(
        rising_dispatches
            .into_iter()
            .map(|target_id| crate::ai::Stimulus::with_human(event_type, target_id.index())),
    );
    stimuli
}

impl EngineInner {
    /// Original: `RHArtificialMalignity::AttackingReactiontimeEnemyNearTest`.
    ///
    /// `RHElementActorSoldier::Hourglass` calls this before the NPC detection
    /// pass. The gate is evaluated once, then the current `mlistThem` is
    /// walked in order and each eligible nearby enemy is sent through Think.
    pub(crate) fn tick_attacking_reactiontime_enemy_near(
        &mut self,
        assets: &LevelAssets,
        scratch: &SimScratch,
    ) {
        let frame = self.control.frame_counter;
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();

        for npc_id in npc_ids {
            let Some(Entity::Soldier(soldier)) = self.world.entities.get(npc_id) else {
                continue;
            };
            if !soldier.element.active {
                continue;
            }
            let Some(enemy_ai) = soldier.npc.ai_brain.enemy() else {
                continue;
            };
            if !attacking_reactiontime_enemy_near_enabled(
                enemy_ai.combat_trainer,
                enemy_ai.base.current_substate,
                frame,
                enemy_ai.base.frame_when_enemy_detected,
            ) {
                continue;
            }

            let origin = soldier.element.position_map();
            let targets = enemy_ai.list_them.clone();
            let nearby_targets = enemies_near_from_them_list(origin, &targets, |target_handle| {
                let target_view = scratch.ai_entity_views.get(&target_handle);
                if target_view.is_none() {
                    tracing::warn!(
                        npc = npc_id.index(),
                        target = target_handle,
                        "EnemyNear: list_them target has no live AI entity view"
                    );
                }
                target_view.map(|view| (view.position, view.posture))
            });

            for target_handle in nearby_targets {
                let Some(target_id) = self.entity_id_for_index(target_handle) else {
                    tracing::warn!(
                        npc = npc_id.index(),
                        target = target_handle,
                        "EnemyNear: list_them target has no live entity"
                    );
                    continue;
                };
                if !matches!(
                    target_id,
                    EntityId::Pc(_) | EntityId::Soldier(_) | EntityId::Civilian(_)
                ) {
                    tracing::warn!(
                        npc = npc_id.index(),
                        target = ?target_id,
                        "EnemyNear: list_them target is not human"
                    );
                    continue;
                }

                let in_uninterruptible_command = self.is_very_very_busy(npc_id);
                let building_sector =
                    self.world.entities.get(npc_id).and_then(|entity| {
                        self.entity_building_sector(entity.element_data().sector())
                    });
                let Some(entity) = self.world.entities.get(npc_id) else {
                    break;
                };
                let mut ctx = build_ai_context_from_entity(
                    entity,
                    frame,
                    building_sector,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &self.ai.global.all_soldier_handles,
                );
                ctx.in_uninterruptible_command = in_uninterruptible_command;
                let tick_data =
                    self.build_npc_tick_data_for_target(npc_id, scratch, assets, Some(target_id));
                let stimulus = crate::ai::Stimulus::with_human(
                    crate::ai::StimulusType::EventEnemyNear,
                    target_handle,
                );
                self.dispatch_think_with_drain(npc_id, &stimulus, &ctx, &tick_data, assets);
            }
        }
    }

    /// P2a — blip detection: reveal blipped soldiers/civilians/objects
    /// that any PC sees this frame, plus drive the Listen ability's
    /// one-shot reveal + FX-target Heard() callbacks.
    pub(super) fn tick_enemy_ai_blip_detection(
        &mut self,
        assets: &LevelAssets,
        world: &AiWorldView,
    ) {
        use crate::element::Posture;

        const DETECTION_FREQUENCY_BLIP: u32 = 16;
        // SeesBlip base multiplier.
        const BLIP_SUPER_DETECTION: f32 = 1.5;
        // Extra factor when PC is on shoulders.
        const BLIP_ON_SHOULDERS_FACTOR: f32 = 1.3;
        const BLIP_CONE_APERTURE_FACTOR: f32 = 1.0;
        const DISTANCE_LISTEN: f32 = 750.0;
        const TIME_LISTEN_WAIT: u32 = 25;
        // Standard view radius — set at level load from the day/night
        // settings.  Falls back to the default only when the level
        // didn't populate one.
        let svr = if self.ai.standard_view_polygon_radius > 0 {
            self.ai.standard_view_polygon_radius as f32
        } else {
            ai_vision::DEFAULT_VIEW_RADIUS as f32
        };

        // Difficulty modifiers.
        let difficulty_factor = match crate::player_profile::DifficultyLevel::current() {
            crate::player_profile::DifficultyLevel::Easy => {
                crate::player_profile::difficulty_params::EASY_BLIP_DETECTION_RANGE
            }
            crate::player_profile::DifficultyLevel::Medium => 1.0,
            crate::player_profile::DifficultyLevel::Hard => {
                crate::player_profile::difficulty_params::HARD_BLIP_DETECTION_RANGE
            }
        };

        // ── Listen ability frame tick. ──────────────────────
        // Each frame a PC is in `ListenPhase::CountingDown`:
        //
        //  - Call `position_iface.turn()` so the PC can still
        //    rotate in place.
        //  - Arm `listen_wait_time` to `TIME_LISTEN_WAIT` on the
        //    first observation.
        //  - Decrement the countdown.  On the frame it reaches 0,
        //    fire the one-shot blip reveal + FX-target `Heard()`
        //    callback (below) and advance the phase to
        //    `ExitTransition` so `tick_abilities` plays the exit
        //    transition animation and cleans up the ability.
        //
        // The action state stays `Listening` through the
        // countdown — the exit transition in `tick_abilities`
        // will flip it back to `Waiting`.
        #[derive(Clone, Copy)]
        struct FiringListener {
            position: MapPoint,
            layer: u16,
            position_z: f32,
            pc_id: EntityId,
        }
        let mut firing_listeners: Vec<FiringListener> = Vec::new();
        let next_order_id = &mut self.orders.next_order_id;
        for &pc_id in &self.world.pc_ids {
            let Some(Entity::Pc(pc)) = self.world.entities.get_mut(pc_id) else {
                continue;
            };
            if pc.actor.listen_phase != crate::element::ListenPhase::CountingDown {
                continue;
            }
            // Advance rotation toward `direction_goal` one step.
            // PI is the source of truth for direction now — no
            // element-side sync needed.
            pc.element.sprite.position_iface.turn();
            if pc.actor.listen_wait_time == 0 {
                // First frame in the CountingDown phase — arm the
                // countdown.
                pc.actor.listen_wait_time = TIME_LISTEN_WAIT;
                continue;
            }
            pc.actor.listen_wait_time -= 1;
            if pc.actor.listen_wait_time != 0 {
                continue;
            }
            // Countdown hit 0 — fire the one-shot reveal and
            // advance the phase so `tick_abilities` plays the
            // exit transition next.
            let fl = FiringListener {
                pc_id,
                position: pc.element.position_map(),
                layer: pc.element.layer(),
                position_z: pc.element.position().z,
            };
            pc.actor.listen_phase = crate::element::ListenPhase::ExitTransition;
            // Bump order_id so the exit transition animation
            // starts fresh in `perform_action`.
            pc.actor.active_ability.order_id =
                Some(crate::abilities::next_listen_order_id(next_order_id));
            firing_listeners.push(fl);
            tracing::debug!(
                pc = pc_id.index(),
                "Listen: one-shot reveal fired after TIME_LISTEN_WAIT frames"
            );
        }

        let sight_obstacles = self.sight_obstacles(assets);
        let mut to_reveal: Vec<EntityId> = Vec::new();
        // Perched PCs that saw an enemy this frame via Path A
        // (SeesBlip) — trigger `HERO_PERCHED_AND_SEE_ENNEMY` speech
        // after the reveal loop.
        let mut perched_detection_triggers: Vec<EntityId> = Vec::new();
        // FX targets within listening range; `Heard(pc)` gets
        // invoked on each below.  Pair: (target_id, listening_pc_id)
        // so we can pass the PC handle to
        // `IElementTargetScript::ActivatedByListenable`.
        let mut to_hear: Vec<(EntityId, EntityId)> = Vec::new();

        // ── FX target Heard() check. ─────────────────────
        // Independent of the blip state — targets are always
        // eligible for Heard() regardless of `blipped`.
        if !firing_listeners.is_empty() {
            for (entity_id, target) in self.world.entities.targets() {
                let target_pos = target.element.position_map();
                let target_layer = target.element.layer();
                let target_z = target.element.position().z;
                for pc in &firing_listeners {
                    if pc.layer != target_layer {
                        continue;
                    }
                    let dx = target_pos.x - pc.position.x;
                    let dy = (target_pos.y - pc.position.y)
                        * crate::position_interface::INVERSE_ASPECT_RATIO;
                    let dz = target_z - pc.position_z;
                    let dist_3d_sq = dx * dx + dy * dy + dz * dz;
                    if dist_3d_sq < DISTANCE_LISTEN * DISTANCE_LISTEN {
                        to_hear.push((entity_id.into(), pc.pc_id));
                        break;
                    }
                }
            }
        }

        for (entity_id, entity) in self
            .world
            .entities
            .npcs()
            .map(|(id, entity)| (id.into(), entity))
            .chain(
                self.world
                    .entities
                    .objects()
                    .map(|(id, entity)| (id.into(), entity)),
            )
        {
            let elem = entity.element_data();

            if !elem.blipped {
                continue;
            }
            let is_npc = entity.npc_data().is_some(); // soldier or civilian
            let is_object = entity.is_object();

            // Royalist soldiers: auto-reveal.
            if entity.is_soldier()
                && let Entity::Soldier(s) = entity
                && s.soldier.cached_camp == Camp::Royalists
            {
                to_reveal.push(entity_id);
                continue;
            }

            // Frame gate for SeesBlip path (NPC-side, every 16 frames).
            // The frame counter is offset by the entity's creation
            // order to stagger NPC detection across 16 frames.
            // EntityId (monotonic slot index, never reused) stands in
            // for that creation counter directly.
            let modified_frame = self.control.frame_counter.wrapping_add(entity_id.index());
            let sees_blip_gate = is_npc && modified_frame.is_multiple_of(DETECTION_FREQUENCY_BLIP);

            // Listen path only fires on the frame a listening PC's
            // countdown hit 0 — `firing_listeners` is non-empty
            // only for that single frame.
            let listen_gate = !firing_listeners.is_empty();

            // Path C (object-side RefreshDiscovered) fires *every*
            // frame for blipped bonuses / scrolls; there is no
            // 16-frame gate here.
            let object_gate = is_object;

            // Skip if no detection path can fire this frame.
            if !sees_blip_gate && !listen_gate && !object_gate {
                continue;
            }

            let blip_pos = elem.position_map();
            let blip_layer = elem.layer();
            let (blip_eye_xy, blip_eye_z) = if entity.is_human() {
                human_eye_point_for_visibility(entity)
            } else {
                (blip_pos, elem.position().z)
            };

            let mut revealed = false;

            // ── Path A: SeesBlip ─────────────────────────────
            if sees_blip_gate {
                for pc in &world.pcs {
                    if pc.layer != blip_layer {
                        continue;
                    }
                    let dx = blip_eye_xy.x - pc.eye_position.x;
                    let dy = (blip_eye_xy.y - pc.eye_position.y)
                        * crate::position_interface::INVERSE_ASPECT_RATIO;
                    let dz = blip_eye_z - pc.eye_z;

                    let super_det = if pc.posture == Posture::OnShoulders {
                        BLIP_SUPER_DETECTION * BLIP_ON_SHOULDERS_FACTOR
                    } else {
                        BLIP_SUPER_DETECTION
                    } * difficulty_factor;

                    let in_range = if dz >= 0.0 {
                        // Blip is higher — 3D spherical check.
                        let dist_3d_sq = dx * dx + dy * dy + dz * dz;
                        dist_3d_sq < super_det * super_det * svr * svr
                    } else {
                        // Blip is lower — 2D cone widens with height.
                        let dist_2d_sq = dx * dx + dy * dy;
                        let h_range = super_det * (svr + BLIP_CONE_APERTURE_FACTOR * (-dz));
                        dist_2d_sq < h_range * h_range
                    };

                    if in_range
                        && crate::sight_obstacle::is_reachable_3d(
                            sight_obstacles,
                            [pc.eye_position.x, pc.eye_position.y, pc.eye_z],
                            [blip_eye_xy.x, blip_eye_xy.y, blip_eye_z],
                            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
                        )
                    {
                        revealed = true;
                        // SeesBlip fires HERO_PERCHED_AND_SEE_ENNEMY
                        // whenever the detecting PC is perched on
                        // shoulders.  Defer the call so we can emit
                        // it after releasing the immutable
                        // `self.world.entities` borrow.
                        if pc.posture == Posture::OnShoulders {
                            perched_detection_triggers.push(pc.id);
                        }
                        break;
                    }
                }
            }

            // ── Path B: ListenTo ─────────────────────────────
            // Simple 3D distance check, no LOS, no cone.  One-shot.
            if !revealed && listen_gate {
                for pc in &firing_listeners {
                    if pc.layer != blip_layer {
                        continue;
                    }
                    let dx = blip_pos.x - pc.position.x;
                    let dy = (blip_pos.y - pc.position.y)
                        * crate::position_interface::INVERSE_ASPECT_RATIO;
                    let dz = elem.position().z - pc.position_z;
                    let dist_3d_sq = dx * dx + dy * dy + dz * dz;
                    if dist_3d_sq < DISTANCE_LISTEN * DISTANCE_LISTEN {
                        revealed = true;
                        break;
                    }
                }
            }

            // ── Path C: object RefreshDiscovered ───────────────
            // For every alive/conscious/active PC, compute the 3D
            // Y-stretched squared distance from the PC's eye point
            // to the bonus and reveal when it drops below
            // `super_detection × svr²` AND the opaque-LOS test
            // passes.  The detection constants are different from
            // the NPC SeesBlip path above — 1.0 (base) or 1.3 (on
            // shoulders), multiplied against `svr²` *before*
            // squaring, so the linear threshold is ≈ svr or
            // 1.14 × svr rather than the 1.5× / 1.95× of NPC
            // SeesBlip.
            //
            // Runs unconditionally (no DETECTION_FREQUENCY_BLIP
            // gate) — it is called from every Hourglass tick.
            // `pc_snapshots` already filters out dead PCs (at
            // snapshot-build time), so we only need to skip
            // unconscious PCs here — `able_to_fight = !unconscious`
            // covers that check.
            if !revealed && object_gate {
                const ON_SHOULDERS_DET: f32 = 1.3;
                const DEFAULT_DET: f32 = 1.0;
                for pc in &world.pcs {
                    if !pc.able_to_fight {
                        // Skip unconscious PCs.
                        continue;
                    }
                    if pc.layer != blip_layer {
                        continue;
                    }
                    let dx = blip_eye_xy.x - pc.eye_position.x;
                    let dy = (blip_eye_xy.y - pc.eye_position.y)
                        * crate::position_interface::INVERSE_ASPECT_RATIO;
                    let dz = blip_eye_z - pc.eye_z;
                    let dist_3d_sq = dx * dx + dy * dy + dz * dz;
                    let super_det = if pc.posture == Posture::OnShoulders {
                        ON_SHOULDERS_DET
                    } else {
                        DEFAULT_DET
                    };
                    if dist_3d_sq < super_det * svr * svr
                        && crate::sight_obstacle::is_reachable_3d(
                            sight_obstacles,
                            [pc.eye_position.x, pc.eye_position.y, pc.eye_z],
                            [blip_eye_xy.x, blip_eye_xy.y, blip_eye_z],
                            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
                        )
                    {
                        revealed = true;
                        break;
                    }
                }
            }

            if revealed {
                to_reveal.push(entity_id);
            }
        }

        for entity_id in to_reveal {
            if let Some(entity) = self.world.entities.get_mut(entity_id) {
                tracing::debug!(
                    entity = entity_id.index(),
                    "reveal_blip: shadow revealed by blip detection"
                );
                entity.reveal_blip();
            }
        }

        // Fire "I see an enemy from my perch" voice lines for any
        // on-shoulders PC that spotted a blip this frame.
        // The anti-chorus timer inside `hero_speaking` absorbs
        // duplicates if multiple blips land on the same perched PC.
        for pc_id in perched_detection_triggers {
            self.hero_speaking(
                assets,
                pc_id,
                crate::engine::melee::HERO_PERCHED_AND_SEE_ENNEMY,
            );
        }

        // Fire FX target Heard() callbacks.  If the target's action
        // filter has `RHFILTER_LISTEN` set AND scripts are enabled,
        // clear the bit and invoke the `ActivatedByListenable(pc)`
        // script callback on the target's own VM.
        //
        // Scripts are always enabled at runtime here (no headless
        // mode) so the script gate is effectively always true — if
        // and when a `--no-script` CLI flag is plumbed, add a check
        // on `GlobalOptions::script_enabled`.
        //
        // Collect (target_handle, pc_handle) pairs first so we can
        // release the mutable entity borrow before dispatching to
        // the mission script (which needs its own engine state
        // swap).
        let mut listenable_calls: Vec<(i32, i32)> = Vec::new();
        for (target_id, listening_pc) in to_hear {
            if let Some(Entity::Target(t)) = self.world.entities.get_mut(target_id)
                && t.target
                    .action_filter
                    .contains(crate::element::TargetFilter::LISTEN)
            {
                t.target
                    .action_filter
                    .remove(crate::element::TargetFilter::LISTEN);
                if !t.target.script_class.is_empty() {
                    let target_handle = crate::natives::GameHost::actor_handle(target_id);
                    let pc_handle = crate::natives::GameHost::actor_handle(listening_pc);
                    listenable_calls.push((target_handle, pc_handle));
                }
            }
        }
        if !listenable_calls.is_empty() {
            self.refresh_script_sight_bindings();
            let queries = native_query_views!(self);
            if let Some(ref mut script) = self.mission_script {
                script.swap_engine_state(
                    &mut self.world.entities,
                    &mut self.ai.global,
                    &mut self.world.fast_grid,
                    &mut self.mission_domain.campaign,
                    &mut self.mission_domain.mission_stat,
                );
                for (target_handle, pc_handle) in listenable_calls {
                    if let Err(e) = script.call_target_function(
                        target_handle,
                        "ActivatedByListenable",
                        &[pc_handle],
                        queries,
                    ) {
                        tracing::warn!("ActivatedByListenable (target {target_handle}): {e}");
                    }
                }
                script.swap_engine_state(
                    &mut self.world.entities,
                    &mut self.ai.global,
                    &mut self.world.fast_grid,
                    &mut self.mission_domain.campaign,
                    &mut self.mission_domain.mission_stat,
                );
            }
            self.sync_game_host_post_script(assets);
        }
    }

    /// Acoustic portion of one NPC's `RefreshDetection` call.
    ///
    /// The hearing branch is called per-tick from every NPC's
    /// `Hourglass`, so civilians run it too — which is how they
    /// react to the PC walking by / swordfighting nearby.
    ///
    /// This stays separate from the soldier-only visual helper so civilians
    /// continue to hear PCs. It is nevertheless called from the creation-
    /// ordered per-NPC coordinator: original `UpdateHearing` invokes
    /// `Think(EVENT_HEAR)` inline, and that state change is visible to the
    /// same NPC's optical `InstantDetection` decision immediately afterward.
    pub(super) fn tick_enemy_ai_acoustic_detection_for_npc(
        &mut self,
        npc_id: EntityId,
        assets: &LevelAssets,
        world: &AiWorldView,
    ) {
        use crate::ai::AiState;

        // Constant 1.0 hearing factor — the static default, never
        // written by shipped code.
        const HEARING_FACTOR: f32 = 1.0;
        const DETECTION_FREQUENCY_SOUNDS: u32 = 3;

        let universal_frame = self.control.frame_counter;
        // Read NPC state. The state gate is sampled once before the enemy-list
        // loop, as in the original outer
        // `if (mCurrentState != STATE_ATTACKING)`.
        let (position, elevation, current_state, expects_pc_detectables) = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            // Every NPC runs the acoustic pass — it lives on the
            // base NPC class.  `expects_pc_detectables` captures
            // the camp-level predicate "does this NPC's enemy
            // list include PCs?"  Royalists iterate the pass but
            // skip PCs they don't track (their inner loop
            // iterates detectable_lists and finds none).
            let expects_pc_detectables = match entity {
                Entity::Civilian(_) => true,
                Entity::Soldier(s) => s.soldier.cached_camp == Camp::Lacklandists,
                _ => return,
            };
            if entity.is_dead() || entity.element_data().posture == Posture::Tied {
                return;
            }
            if entity.human_data().map(|h| h.unconscious).unwrap_or(false) {
                return;
            }
            let Some(npc) = entity.npc_data() else {
                return;
            };
            (
                entity.element_data().position_map(),
                entity.element_data().position().z,
                npc.ai_state(),
                expects_pc_detectables,
            )
        };
        // Attacking NPCs are already locked onto their target
        // and don't accumulate new hearing stimuli.
        if matches!(current_state, AiState::Attacking) {
            return;
        }
        let modified_frame = universal_frame.wrapping_add(npc_id.index());
        if !modified_frame.is_multiple_of(DETECTION_FREQUENCY_SOUNDS) {
            return;
        }

        // Fold the max covering volume from active sound sources
        // at the NPC's position into the deafness write-back.
        // Computed here because `NpcData` has no access to the
        // `SoundSourceManager`.  Done before the entity re-borrow
        // so we don't hold `&mut self.world.entities` while reading
        // `&self.feedback.sound_sim`.
        let cover_volume = self
            .feedback
            .sound_sim
            .sources
            .max_noise_covering_volume_for_3d(position.x, position.y, elevation);

        let (deafness, pc_target_ids) = {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            let Some(npc) = entity.npc_data_mut() else {
                return;
            };
            let enemy_idx = DetectableType::Enemy as usize;

            // Lazy-populate: civilians + Lacklandist soldiers were
            // initialised with the level's initial PC roster, but
            // late-spawned PCs (reinforcements via bootstrap script)
            // arrive after InitOneAI.  The runtime `AddDetectable`
            // path only adds PCs to NPCs whose `AddDetectable` class
            // filter passes — Royalist soldiers do NOT track PCs
            // (they only track Lacklandist enemies), so we skip the
            // populate for them.
            if expects_pc_detectables {
                for pc in &world.pcs {
                    if !npc.detectable_lists[enemy_idx]
                        .iter()
                        .any(|d| d.element == Some(pc.id))
                    {
                        npc.detectable_lists[enemy_idx].push(Detectable {
                            element: Some(pc.id),
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
            }

            let deafness = npc.get_deafness(universal_frame, cover_volume) as f32;
            // `RefreshDetection` walks this NPC's DETECTABLE_ENEMY list, not
            // the engine PC registry. Preserve that list's insertion order:
            // each inline Think may mutate state observed by the next entry.
            let pc_target_ids: Vec<EntityId> = npc.detectable_lists[enemy_idx]
                .iter()
                .filter_map(|detectable| match detectable.element {
                    Some(id @ EntityId::Pc(_)) => Some(id),
                    _ => None,
                })
                .collect();
            (deafness, pc_target_ids)
        };

        let enemy_idx = DetectableType::Enemy as usize;
        for pc_id in pc_target_ids {
            let Some(pc) = world.pcs.iter().find(|pc| pc.id == pc_id) else {
                // A dead or inactive PC can remain in the
                // detectable list until optical CleanUpDetectables later in
                // this same RefreshDetection call. There is no acoustic
                // snapshot to sample in that expected stale window. A live
                // registered PC missing from the world view is inconsistent.
                match self.world.entities.get(pc_id) {
                    Some(entity) if !entity.is_active() || entity.is_dead() => continue,
                    Some(_) | None => panic!(
                        "NPC {} tracks live PC {} for hearing but the PC is absent from the detection view",
                        npc_id.index(),
                        pc_id.index()
                    ),
                }
            };
            let stimulus = {
                let Some(entity) = self.world.entities.get_mut(npc_id) else {
                    return;
                };
                let Some(npc) = entity.npc_data_mut() else {
                    return;
                };
                // RefreshDetection iterates `DETECTABLE_ENEMY` and
                // filters PCs.  Skip PCs absent from this NPC's list
                // (Royalists don't track PCs, so they naturally hear
                // nothing here).
                let tracked = npc.detectable_lists[enemy_idx]
                    .iter()
                    .any(|d| d.element == Some(pc.id));
                if !tracked {
                    None
                } else {
                    let pc_volume = pc.noise_volume;
                    // Hear-my-noise-box pre-filter: half-extents are
                    // (volume + 100, volume*ASPECT_RATIO + 100) in raw
                    // map coords. Outside this box original RefreshDetection
                    // does not call UpdateHearing, so the latch is untouched.
                    let dx = pc.position.x - position.x;
                    let dy_raw = pc.position.y - position.y;
                    let half_x = pc_volume as f32 + 100.0;
                    let half_y = pc_volume as f32 * crate::position_interface::ASPECT_RATIO + 100.0;
                    if dx.abs() > half_x || dy_raw.abs() > half_y {
                        None
                    } else {
                        // GetHearVolume uses the full 3D position. Its noise
                        // origin is `(x, y + elevation, elevation)` and it has
                        // no logical-layer rejection, so nearby cross-layer
                        // sounds remain audible when their actual geometry is.
                        let source_elevation = pc.ground_elevation as f32;
                        let dy_stretched = (position.y - pc.position.y - source_elevation)
                            * crate::position_interface::INVERSE_ASPECT_RATIO;
                        let dx_3d = position.x - pc.position.x;
                        let dz = elevation - source_elevation;
                        let modified_volume = pc_volume as f32 * HEARING_FACTOR;
                        let max_norm = dx_3d.abs().max(dy_stretched.abs()).max(dz.abs());
                        let distance =
                            (dx_3d * dx_3d + dy_stretched * dy_stretched + dz * dz).sqrt();
                        // Original GetHearVolume explicitly rejects NOISE_OFF,
                        // a coincident source/listener, and sources beyond the
                        // modified-volume max norm. UpdateHearing still runs
                        // for all of these inside-box cases and clears its
                        // rising-edge latch.
                        let subjective =
                            if pc_volume == 0 || distance == 0.0 || max_norm > modified_volume {
                                0.0
                            } else {
                                (modified_volume - distance - deafness).max(0.0)
                            };

                        let (det_heard, det_seen) = npc.detectable_lists[enemy_idx]
                            .iter()
                            .find(|d| d.element == Some(pc.id))
                            .map(|d| (d.heard_last_frame, d.seen_last_frame))
                            .unwrap_or_else(|| {
                                panic!(
                                    "tracked PC {} disappeared from NPC {}'s enemy list",
                                    pc.id.index(),
                                    npc_id.index()
                                )
                            });

                        let stimulus = if subjective > 0.0 && !det_heard && !det_seen {
                            let noise_type = if pc.is_swordfighting {
                                crate::ai::NoiseType::ZingZing
                            } else {
                                crate::ai::NoiseType::TapTapTap
                            };
                            let noise = crate::ai::Noise {
                                origin: crate::ai::Position {
                                    x: pc.position.x,
                                    y: pc.position.y,
                                    sector: crate::position_interface::SectorHandle::new(
                                        pc.sector_num,
                                    ),
                                    level: pc.layer,
                                },
                                noise_type,
                                volume: subjective as u16,
                                elevation: pc.ground_elevation,
                                element_id: pc.id.index() as u16,
                            };
                            Some(crate::ai::Stimulus::with_noise(
                                crate::ai::StimulusType::EventHear,
                                noise,
                            ))
                        } else {
                            None
                        };

                        // UpdateHearing always refreshes this latch when the
                        // hear-box admitted the target, including zero-volume
                        // and beyond-range cases.
                        let det = npc.detectable_lists[enemy_idx]
                            .iter_mut()
                            .find(|d| d.element == Some(pc.id))
                            .expect("hearing detectable vanished between reads");
                        det.heard_last_frame = subjective > 0.0;
                        stimulus
                    }
                }
            };

            let Some(stimulus) = stimulus else {
                continue;
            };

            // `UpdateHearing` calls Think inline. Rebuild both views for each
            // edge because an earlier PC's hearing handler may mutate state
            // consumed by the next handler or by optical detection below.
            let scratch = self.build_sim_scratch(assets);
            let in_uninterruptible_command = self.is_very_very_busy(npc_id);
            let building_sector = self
                .world
                .entities
                .get(npc_id)
                .and_then(|entity| self.entity_building_sector(entity.element_data().sector()));
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let mut ctx = build_ai_context_from_entity(
                entity,
                self.control.frame_counter,
                building_sector,
                self.world.weather.is_forest_level,
                self.world.weather.ambiance,
                self.ai.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &self.ai.global.all_soldier_handles,
            );
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            let tick_data = self.build_npc_tick_data(npc_id, &scratch, assets);
            self.dispatch_think_with_drain(npc_id, &stimulus, &ctx, &tick_data, assets);
        }
    }

    /// P3 — per-enemy `RefreshDetection` pass.
    ///
    /// For every Lacklandist NPC: lazy-populate detectables, run the
    /// synchronous acoustic EVENT_HEAR pass, run per-target visibility,
    /// accumulate suspect sharpness, queue EVENT_SEES_SHADOW and EVENT_VIEW,
    /// then run the remaining detectable buckets and flush their FIFO stimuli
    /// for that same NPC before advancing to the next creation slot. EVENT_VIEW
    /// is queued after the Enemy scan and dispatched with a live boundary
    /// context only after every detectable bucket has released the NPC borrow.
    /// Volatile NPC target metadata is rebuilt at each creation slot so a
    /// later NPC observes state changes made by an earlier NPC's Think.
    /// Original:
    /// `RHelementactornpc.cpp::RefreshDetection` queues detection stimuli while
    /// scanning lists, then calls `Think` before returning from that NPC's
    /// Hourglass. Returns the rising/falling edges for later phases.
    // TODO(parity): EVENT_OUTOFVIEW is collected for a later global dispatch;
    // Royalist detection remains a later global phase; and every stimulus in
    // one NPC's FIFO shares the same boundary scratch instead of rebuilding
    // context after each preceding Think mutation.
    pub(super) fn tick_enemy_ai_refresh_detection(
        &mut self,
        assets: &LevelAssets,
        world: &AiWorldView,
    ) -> (Vec<Detection>, Vec<(EntityId, u32)>) {
        let mut transitions: Vec<Detection> = Vec::new();
        // Falling-edge EVENT_OUTOFVIEW queue: per-detectable
        // (npc_id, target_handle) pairs whose `seen_last_frame` just
        // transitioned to false.  Drained at the end of the detection
        // pass, after the outer NPC borrow ends.
        let mut out_of_view_dispatches: Vec<(EntityId, u32)> = Vec::new();

        let universal_frame = self.control.frame_counter;
        let golden_eye = self.ai.global.golden_eye_mode;
        // Forest-level flag — selects between forest and city
        // detection-speed parameters when scaling a PC's visual
        // detection speed in the per-target visibility pass below.
        let is_forest_level = self.world.weather.is_forest_level;
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();

        for npc_id in npc_ids {
            self.tick_enemy_ai_acoustic_detection_for_npc(npc_id, assets, world);
            let think_input = self.tick_enemy_ai_refresh_detection_for_npc(
                npc_id,
                assets,
                world,
                universal_frame,
                golden_eye,
                is_forest_level,
                &mut transitions,
                &mut out_of_view_dispatches,
            );
            // Enemy HandlePredetection may already have queued
            // EVENT_SEES_SHADOW. Append the committed ENEMY stimulus now,
            // before scanning later detectable types, preserving the relative
            // SHADOW → VIEW → BODY → OBJECT → FRIEND → MISSED_FRIEND → BEGGAR
            // order behind any earlier stimulus already in the NPC FIFO.
            let event_view_tick_data = if let Some((stimulus, tick_data)) = think_input {
                let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                    panic!(
                        "detected NPC {} disappeared before its same-phase stimulus queue",
                        npc_id.index()
                    )
                });
                let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                    panic!(
                        "detected NPC {} lost its AI controller before stimulus queue",
                        npc_id.index()
                    )
                });
                let queue_index = ai.pending_stimuli.len();
                let stimulus_type = stimulus.stimulus_type;
                ai.pending_stimuli.push(stimulus);
                Some(super::post_detection::PendingEventViewTickData {
                    queue_index,
                    stimulus_type,
                    tick_data,
                })
            } else {
                None
            };
            // Original NPC::Hourglass completes this NPC's entire
            // RefreshDetection scan before flushing its FIFO stimulus list.
            // Rebuild only the volatile human/object target metadata here;
            // `world.pcs` remains the once-per-frame snapshot because its
            // construction also updates produced-noise state. No Think has
            // run for this NPC yet, so all its buckets observe the same
            // pre-Think state.
            let (human_targets, object_targets) =
                self.tick_enemy_ai_build_human_object_targets_for_npc(npc_id);
            self.tick_enemy_ai_refresh_per_type_for_npc(
                npc_id,
                assets,
                &human_targets,
                &object_targets,
                universal_frame,
                golden_eye,
            );

            let has_pending_stimuli = self
                .world
                .entities
                .get(npc_id)
                .and_then(Entity::ai_controller)
                .is_some_and(|ai| !ai.pending_stimuli.is_empty());
            if !has_pending_stimuli {
                assert!(
                    event_view_tick_data.is_none(),
                    "queued EVENT_VIEW lost its pending stimulus before the per-NPC drain"
                );
                continue;
            }

            // Refresh live entity/context views at this NPC's FIFO-flush
            // boundary. Earlier NPC Think calls may have changed AI state,
            // focus, sequences, or other metadata consumed by this slot.
            // Quiet NPCs skip this full entity-view clone entirely.
            let live_scratch = self.build_sim_scratch(assets);

            // TODO(parity): rebuild context views between successive queued
            // optical stimuli when an earlier Think can mutate data consumed
            // by the next one. Royalist detection is still coordinated by its
            // existing global phase rather than this per-NPC loop.
            self.tick_enemy_ai_drain_pending_stimuli_for_npc(
                npc_id,
                assets,
                &live_scratch,
                event_view_tick_data,
            );
        }

        (transitions, out_of_view_dispatches)
    }

    /// P3 inner — per-NPC body of [`Self::tick_enemy_ai_refresh_detection`].
    /// Carries the per-NPC tracing span so all events emitted inside the
    /// detection pass automatically include `npc=<id>` in their span context.
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    fn tick_enemy_ai_refresh_detection_for_npc(
        &mut self,
        npc_id: EntityId,
        assets: &LevelAssets,
        world: &AiWorldView,
        universal_frame: u32,
        golden_eye: bool,
        is_forest_level: bool,
        transitions: &mut Vec<Detection>,
        out_of_view_dispatches: &mut Vec<(EntityId, u32)>,
    ) -> Option<(crate::ai::Stimulus, AiPerTickData)> {
        use crate::ai::AiState;
        use crate::element::{ActionState, Posture};

        let pc_snapshots = world.pcs.as_slice();
        let soldier_snapshots = world.soldiers.as_slice();
        let ko_money_fight_soldiers = world.ko_money_fight_soldiers.as_slice();
        let primary_target_multiplicity = &world.primary_target_multiplicity;
        let pc_forecasts = &world.pc_forecasts;
        let npc_jump_lines = &world.npc_jump_lines;

        // -- Read enemy state in a scoped borrow --
        let viewer = {
            let entity = self.world.entities.get(npc_id)?;
            SoldierSightContext::from_viewer(entity, Camp::Lacklandists)?
        };
        if viewer.ai_locked {
            return None;
        }
        let eye = viewer.eye;
        let eye_z = viewer.eye_z;
        let dir = viewer.dir;
        let layer = viewer.layer;
        let view_radius = viewer.view_radius;
        let eye_status = viewer.eye_status;
        let current_state = viewer.current_state;
        let view_forward = viewer.view_forward;
        let real_half_aperture = viewer.real_half_aperture;
        let npc_posture = viewer.posture;
        let entity_sector = viewer.sector;
        let alert_status = viewer.alert_status;
        let viewer_blipped = viewer.blipped;
        let me_pos_map = viewer.position_map;
        // Silence the "unused" warning on the `_action_state` slot
        // we keep for readability of the destructure pattern.
        let _ = ActionState::Waiting;

        // Resolve the viewer's building sector from the entity's
        // cached sector (set during door-pass transitions).  Used by
        // RefreshDetection / IsDetecting to short-circuit visibility
        // when the viewer is indoors.
        let viewer_building_sector = self.entity_building_sector(entity_sector);

        // Compute effective view radius accounting for eye height
        // and night/fog light modulation.  Computed once per NPC
        // (the ground path is cached as a last-viewed-radius
        // ground value).
        let is_night_or_fog = matches!(
            self.world.weather.ambiance,
            crate::engine::types::Ambiance::Night | crate::engine::types::Ambiance::Fog
        );
        // Once-per-viewer base call — the ground (no-obstacle) radius.
        // Used as the fast-path for any target that is not standing on
        // a projection obstacle.  Targets with `obstacle_idx = Some`
        // get a per-target re-call below, so the night/fog modulation
        // accounts for the target's elevation.
        let effective_view_radius_ground = ai_vision::compute_view_radius(
            eye,
            eye_z,
            view_radius,
            view_forward,
            real_half_aperture,
            is_night_or_fog,
            &self.world.fast_grid.level,
            self.sight_obstacles(assets),
            None,
        );
        // Per-NPC frame-counter phase offset so not every NPC
        // re-runs detection on the same tick.  EntityId (monotonic
        // slot index, never reused) stands in for the creation
        // counter directly.
        let modified_frame = universal_frame.wrapping_add(npc_id.index());
        // Gate fires when the modified frame counter aligns with
        // `DETECTION_FREQUENCY_ENEMY_PC`.  `refresh_always` is true
        // when eye status is Stare / Follow or when alert_status is
        // anything other than Green — that bypasses the per-NPC gate
        // so a staring / on-alert NPC refreshes visibility every
        // tick instead of only on the gate-open frame.
        let refresh_always = matches!(
            eye_status,
            crate::element::EyeStatus::Stare | crate::element::EyeStatus::Follow
        ) || !matches!(alert_status, crate::ai::AlertLevel::Green);
        let gate_open = refresh_always
            || modified_frame.is_multiple_of(ai_vision::DETECTION_FREQUENCY_ENEMY_PC);
        // InstantDetection for Lacklandist enemies: false when the
        // NPC is sleeping / on patrol / wondering, true when already
        // seeking / attacking / menacing / fleeing.
        let instant_detection = !matches!(
            current_state,
            AiState::Sleeping | AiState::Default | AiState::Wondering
        );

        // -- Mutating pass: update detectable list + suspects --
        // `&self.sight_obstacles` and `self.world.entities.get_mut(...)`
        // are disjoint fields on `self`, so the split borrow is
        // valid.  We scope the mut access so we can push to
        // `transitions` afterwards without conflict.
        let mut commit: Option<(EntityId, MapPoint, bool)> = None;
        let mut think_tick_data: Option<AiPerTickData> = None;
        let mut think_stimulus: Option<crate::ai::Stimulus> = None;
        {
            // Build the obstacle view from individual disjoint
            // fields so the borrow checker can split it from the
            // mut borrows of `ai_global` / `entities` below. Going
            // through `engine.sight_obstacles(assets)` would be a
            // method-level borrow of `self`, not field-level.
            let sight_obstacles = crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.static_sight_obstacles.as_slice(),
                dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                static_active: &self.world.static_sight_obstacle_active,
            };
            // Split-borrow `ai_global` so we can pass it into
            // `EnemyAi::think` alongside the mut borrow on
            // `self.world.entities`.  Rust field-level borrow checking
            // allows this because they're disjoint fields.  The
            // outer `ai_global` split-borrow is only read by a
            // nested scope below; the now-deferred stimulus pushes
            // at this level don't need it.
            let _ai_global = &mut self.ai.global;
            let Some(Entity::Soldier(soldier)) = self.world.entities.get_mut(npc_id) else {
                return None;
            };

            // Beggar-trick learning.  Capture the AI's current
            // `got_the_beggar_trick` flag before taking a mut borrow
            // on `detectable_lists` (both fields live under
            // `soldier.npc`).  We mutate a local during the loop and
            // write back after the borrow on `detectables` releases.
            let mut got_beggar_trick = soldier
                .npc
                .ai_brain
                .base()
                .map(|ai| ai.got_the_beggar_trick)
                .unwrap_or(false);

            let enemy_idx = DetectableType::Enemy as usize;
            let detectables: &mut Vec<Detectable> = &mut soldier.npc.detectable_lists[enemy_idx];

            // Lazy-populate: ensure every currently-alive PC has a
            // Detectable entry.  The level loader doesn't know about
            // the final PC roster at soldier-init time (PCs are
            // registered later through the mission-script
            // bootstrap), so we reconcile on the first tick that
            // has a populated `pc_snapshots`; subsequent ticks
            // short-circuit on the `iter().any(...)` check.
            for pc in pc_snapshots {
                if !detectables.iter().any(|d| d.element == Some(pc.id)) {
                    detectables.push(Detectable {
                        element: Some(pc.id),
                        detectable_type: DetectableType::Enemy,
                        ..Default::default()
                    });
                    tracing::trace!(
                        npc = ?npc_id,
                        target = ?pc.id,
                        "LAZY_POPULATE PC added"
                    );
                }
            }
            // CleanUpDetectables — drop entries whose target is dead
            // or gone.
            let before = detectables.len();
            detectables.retain(|d| {
                d.element
                    .is_some_and(|id| pc_snapshots.iter().any(|p| p.id == id))
            });
            if before != detectables.len() {
                tracing::trace!(
                    npc = ?npc_id,
                    before,
                    after = detectables.len(),
                    "LAZY_POPULATE PC retain dropped entries"
                );
            }

            // Per-target visibility pass.
            //
            // `best_target` tracks the unoccupied-preferred primary
            // target pick — lowest-score wins, where score is the
            // Euclidean distance + a penalty for how many friendly
            // soldiers already target this PC.  We use `u32::MAX`
            // for "no target yet" so the first visible PC always
            // replaces it.
            let mut sum_sharpness_new: u32 = 0;
            let mut any_seen_now = false;
            let mut best_target: Option<(EntityId, MapPoint, u32)> = None;
            let mut max_visibility_raw: f32 = 0.0;

            for det in detectables.iter_mut() {
                let target_id = det
                    .element
                    .expect("enemy detectable survived cleanup without a target entity handle");
                let pc = pc_snapshots
                    .iter()
                    .find(|p| p.id == target_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "enemy detectable target {} is absent from the per-tick PC view",
                            target_id.index()
                        )
                    });

                // Different layer ⇒ different floor in a building;
                // LOS raycast won't cross and the IsActive check
                // would have bailed earlier.
                if pc.layer != layer {
                    det.seen_now = false;
                    det.last_visibility = 0.0;
                    continue;
                }

                // Only call `ComputeVisibility` when the
                // detection-frequency gate is open.  On closed-gate
                // frames the cached post-multiplied value from the
                // most recent gate-open frame is reused, so the
                // sharpness accumulator decays smoothly instead of
                // dropping to 0 every non-gate tick.  The gate-open
                // branch stores the post-multiplied value into
                // `det.last_visibility` (see the assignment after
                // the multiplications below), and the closed-gate
                // branch just reuses it.
                let visibility_raw = if gate_open {
                    // Same-building rule:
                    //   if viewer in building:
                    //     if target in same building AND target
                    //       alive / conscious / NOT passing door → 0.5
                    //     else → 0.0
                    // Dead PCs are filtered upstream at
                    // `pc_snapshots` build-time; unconscious and
                    // door-passing targets are still in the
                    // snapshot and must be gated here.
                    let viewer_in_building = viewer_building_sector.is_some();
                    let target_in_same_building =
                        viewer_in_building && viewer_building_sector == pc.building_sector;
                    // Blipped NPCs standing outside a building cannot
                    // see PCs (the blip overlay occludes their eyes).
                    // Inside-building blipped NPCs still use the
                    // same-building short-circuit above.
                    if viewer_blipped && !viewer_in_building {
                        det.seen_now = false;
                        det.last_visibility = 0.0;
                        continue;
                    }
                    // Posture-based Z offsets for the 3D close-range
                    // distance check (see
                    // `ai_vision::compute_visibility`).  The LOS
                    // raycast itself is still 2D until sight-obstacle
                    // data carries Z.
                    //
                    // Per-target effective view radius accounts for
                    // the target's projection obstacle (e.g. roof /
                    // ledge).  Ground targets reuse the hoisted
                    // `effective_view_radius_ground`.
                    let effective_view_radius = pc
                        .obstacle_idx
                        .and_then(|h| sight_obstacles.get(usize::from(h)))
                        .map(|obs| {
                            ai_vision::compute_view_radius(
                                eye,
                                eye_z,
                                view_radius,
                                view_forward,
                                real_half_aperture,
                                is_night_or_fog,
                                &self.world.fast_grid.level,
                                sight_obstacles,
                                Some(obs),
                            )
                        })
                        .unwrap_or(effective_view_radius_ground);
                    let q = ai_vision::VisibilityQuery {
                        viewer: eye,
                        viewer_direction: dir,
                        view_forward,
                        view_radius,
                        viewer_eye_status: eye_status,
                        real_half_aperture,
                        viewer_in_building,
                        target_in_same_building,
                        // Forest 180° merry-men special case is
                        // for Royalist NPCs only; we iterate
                        // Lacklandists in this loop, so always
                        // false.  The Royalist visibility path in
                        // the npc-targets loop already gates on
                        // `is_forest_level && !is_rider_npc`.
                        forest_180_degree_view: false,
                        golden_eye_mode: golden_eye,
                        effective_view_radius,
                        // PCs in pc_snapshots are always active
                        // (filtered at snapshot build time), so
                        // "active and outside building" reduces to
                        // "not in a building".
                        target_is_active_and_outside_building: pc.building_sector.is_none(),
                        target: crate::stealth::detection_point_xy(
                            pc.position,
                            pc.posture,
                            pc.direction as i16,
                        ),
                        target_posture: pc.posture,
                        target_action_state: pc.action_state,
                        target_is_pc: true,
                        viewer_eye_z: eye_z,
                        target_eye_z: pc.detection_z,
                        sight_obstacles,
                        fast_grid: &self.world.fast_grid,
                        layer,
                        target_unconscious: pc.unconscious,
                        target_passing_door: pc.passing_door,
                    };
                    ai_vision::compute_visibility(&q)
                } else {
                    0.0
                };
                // Multiply by the frequency so that the averaged
                // sharpness over time matches a per-frame call.
                //
                // For PC targets (non-soldier), scale further by the
                // PC's profile-level forest/city detection-speed
                // percentage.  A stealthy hero (e.g. a scout profile
                // with a low detection speed) is slower to spot; a
                // loud hero is faster.  Only apply this inside the
                // refresh gate — the cached `last_visibility` value
                // already has it baked in.
                let mut visibility = if gate_open {
                    let detection_speed_pct = if is_forest_level {
                        pc.detection_speed_in_forest
                    } else {
                        pc.detection_speed_in_city
                    };
                    ai_vision::DETECTION_FREQUENCY_ENEMY_PC as f32
                        * visibility_raw
                        * 0.01
                        * detection_speed_pct as f32
                } else {
                    // Closed-gate frame — reuse the cached post-
                    // multiplied value from the last refresh so the
                    // sharpness accumulator decays smoothly instead
                    // of dropping to 0 every non-gate tick.
                    det.last_visibility
                };

                // "Did you know that a certain Stuteley sometimes
                // dresses up as beggar?"  When the NPC has not yet
                // learned the beggar trick and the PC is currently
                // visible, gate on the PC's running animation:
                //   * SimulatingBeggar (resting beggar pose) → return 0;
                //     the NPC just sees an old beggar, not the disguised
                //     hero.
                //   * Transition WaitingUpright↔SimulatingBeggar (mid-
                //     change) → the NPC catches the swap and learns the
                //     trick (`got_the_beggar_trick = true`).  Visibility
                //     stays > 0 so the sighting still commits this frame.
                // Once the flag is true the NPC sees through future
                // beggar disguises permanently (per-NPC, not global).
                if !got_beggar_trick && visibility > 0.0 {
                    use crate::order::OrderType;
                    match pc.order_type {
                        OrderType::SimulatingBeggar => {
                            visibility = 0.0;
                        }
                        OrderType::TransitionWaitingUprightSimulatingBeggar
                        | OrderType::TransitionSimulatingBeggarWaitingUpright => {
                            got_beggar_trick = true;
                        }
                        _ => {}
                    }
                }

                // Sharpness depends on posture.  Leaning out uses
                // 10x faster detection (200 vs 20).
                let view_speed = if npc_posture == Posture::LeaningOut {
                    ai_vision::LOOK_DOWN_BASE_VIEW_SPEED
                } else {
                    ai_vision::BASE_VIEW_SPEED
                };
                let sharpness = (view_speed as f32 * visibility) as u32;
                let is_visible = sharpness > 0;
                tracing::trace!(
                    npc = ?npc_id,
                    target = ?target_id,
                    gate_open,
                    visibility_raw,
                    visibility,
                    sharpness,
                    is_visible,
                    prev_seen_last_frame = det.seen_last_frame,
                    npc_dir = dir,
                    view_forward_x = view_forward.0,
                    view_forward_y = view_forward.1,
                    real_half_aperture,
                    viewer_x = eye.x,
                    viewer_y = eye.y,
                    target_x = pc.position.x,
                    target_y = pc.position.y,
                    "visibility check"
                );

                // Accumulate sharpness until EVENT_VIEW has been
                // dispatched for this detectable.  `seen_last_frame`
                // is a separate latch that only flips true inside
                // the commit block below.  So long as the target
                // stays visible but hasn't been committed yet,
                // sharpness keeps growing every frame, driving the
                // suspect counter (and the growing question-mark
                // emoticon) toward DETECTION_SUSPECT_THRESHOLD.
                if is_visible && !det.seen_last_frame {
                    sum_sharpness_new = sum_sharpness_new.saturating_add(sharpness);
                }

                if is_visible {
                    any_seen_now = true;
                    // Unoccupied-preferred primary-target scoring:
                    //   distance = Distance(enemy)
                    //   distance += 100 * primary_target_multiplicity
                    //   pick the lowest distance
                    let dx = pc.position.x - eye.x;
                    let dy = pc.position.y - eye.y;
                    let dist_sq = dx * dx + dy * dy;
                    let dist = dist_sq.sqrt() as u32;
                    let mult = primary_target_multiplicity
                        .get(&target_id)
                        .copied()
                        .unwrap_or(0);
                    let score = dist + 100 * mult;
                    let replace = match best_target {
                        None => true,
                        Some((_, _, s)) => score < s,
                    };
                    if replace {
                        best_target = Some((target_id, pc.position, score));
                    }
                }

                // Single-field update.  Next frame's edge-trigger
                // reads this value directly.
                det.seen_now = is_visible;
                // Store the post-frequency / post-detection-speed
                // visibility.  Only update on gate-open frames;
                // closed-gate frames re-read the cached value above
                // without overwriting it.
                if gate_open {
                    det.last_visibility = visibility;
                }
                if visibility_raw > max_visibility_raw {
                    max_visibility_raw = visibility_raw;
                }
            }

            // Write back the beggar-trick flag if a mid-transition
            // sighting flipped it during the loop.
            if got_beggar_trick
                && let Some(ai) = soldier.npc.ai_brain.base_mut()
                && !ai.got_the_beggar_trick
            {
                ai.got_the_beggar_trick = true;
                tracing::trace!(
                    npc = ?npc_id,
                    "got_the_beggar_trick → true (mid-transition sighting)"
                );
            }

            // Acoustic detection moved out of this loop — the
            // shared acoustic-detection pass earlier in
            // `tick_enemy_ai` runs the hearing check for every
            // NPC (civilians + Lacklandist soldiers) instead of
            // just the ones that pass this soldier-visual loop's
            // filter.  Hearing is a shared NPC behaviour, not an
            // enemy-specific one.

            let my_camp = soldier.soldier.cached_camp;
            if let Some(enemy_ai) = soldier.npc.ai_brain.enemy_mut() {
                // Maximum-visibility tracker — used by
                // DefaultLookingShadow to keep watching while the
                // target is still partially visible.
                enemy_ai.base.max_visibility = max_visibility_raw;

                // Pre-resolve target metadata (position, posture,
                // animation) from the pc_snapshots cache when the
                // primary target is a PC. Used by
                // `reconsider_enemy_approach` for live-target reads.
                // Carrier position is left None here — the
                // on-shoulders branch is handled in the separate
                // timer / reach-point dispatch paths that have
                // direct entity access.
                let (primary_target_position, primary_target_posture, primary_target_animation) = {
                    let target_handle = enemy_ai.base.primary_target;
                    if target_handle != 0
                        && let Some(pc) = pc_snapshots
                            .iter()
                            .find(|p| p.id == EntityId::Pc(crate::entity_id::PcId(target_handle)))
                    {
                        (
                            Some(crate::ai::Position {
                                x: pc.position.x,
                                y: pc.position.y,
                                sector: crate::position_interface::SectorHandle::new(pc.sector_num),
                                level: pc.layer,
                            }),
                            Some(pc.posture),
                            Some(pc.order_type),
                        )
                    } else {
                        (None, None, None)
                    }
                };
                // ── Populate combat context from engine ──────
                let mut tick_data = AiPerTickData {
                    profile_manager: Some(assets.profile_manager.clone()),
                    // Pre-computed forecast for the primary target.
                    primary_target_forecast: pc_forecasts
                        .get(&enemy_ai.base.primary_target)
                        .copied(),
                    // pc_forecasts is keyed by PC entity ids only.
                    primary_target_is_pc: pc_forecasts.contains_key(&enemy_ai.base.primary_target),
                    // Pre-computed forecast for the missed PC.
                    missed_pc_forecast: pc_forecasts.get(&enemy_ai.missed_pc).copied(),
                    missed_pc_is_pc: pc_forecasts.contains_key(&enemy_ai.missed_pc),
                    // Table swordfight jump-line for primary target.
                    primary_target_jump_line: npc_jump_lines.get(&npc_id).copied().flatten(),
                    primary_target_position,
                    primary_target_posture,
                    primary_target_animation,
                    // friend_swap_candidates left empty here — the
                    // main tick path holds a mut borrow on the
                    // current soldier, preventing a scan of the
                    // other soldiers' AI state. The timer / reach-
                    // point dispatch paths build candidates and
                    // drive the swap heuristic.
                    ..AiPerTickData::stub()
                };
                // Build them-list: visible enemies with distances.
                //
                // Cleanup pass during battle decisions: an enemy
                // that isn't able to fight gets removed from the
                // them-list, and if they're unconscious and not
                // being carried they're appended to the
                // unconscious-enemies side-list.  We do the same
                // split here so `battle_decisions` can consume
                // `tick_data.unconscious_enemies` directly without
                // walking `list_them` again.
                //
                // The them-list is owned by the AI controller and
                // persists across detection ticks — it's mutated
                // only by reinitialise / end-swordfight / explicit
                // beggar handling.  The engine detection tick
                // therefore must NOT clear `list_them`; it only
                // produces the per-tick visibility metadata that
                // feeds `tick_data` (min distance, unconscious-enemy
                // side list, etc.).  Clearing it here used to empty
                // `list_them` on any frame where the PC's
                // `seen_now` flickered false, which in turn drove
                // `battle_decisions` into its
                // `num_enemies_i_can_see == 0` fallback
                // (stand-and-observe) instead of the intended
                // Fight → approach path.
                tick_data.enemy_sq_distances.clear();
                tick_data.min_sq_enemy_distance = i32::MAX;
                tick_data.seen_last_frame_enemies.clear();
                // Snapshot the `seen_last_frame` flag on every enemy
                // detectable so `RefreshArrowProtection` can gate its
                // dangerous-archer scan on the soldier's own
                // perception.
                for det in soldier.npc.detectable_lists[enemy_idx].iter() {
                    if det.seen_last_frame
                        && let Some(elem) = det.element
                    {
                        tick_data.seen_last_frame_enemies.push(elem.index());
                    }
                }
                for det in soldier.npc.detectable_lists[enemy_idx].iter() {
                    if !det.seen_now {
                        continue;
                    }
                    let Some(target_id) = det.element else {
                        continue;
                    };
                    if let Some(pc) = pc_snapshots.iter().find(|p| p.id == target_id) {
                        if pc.unconscious {
                            // Non-carried unconscious enemies become
                            // finish-off candidates.  Carried PCs
                            // are skipped entirely.
                            if !pc.carried {
                                tick_data
                                    .unconscious_enemies
                                    .push(crate::ai::SleepingEnemyInfo {
                                        handle: target_id.index(),
                                        position: crate::ai::Position {
                                            x: pc.position.x,
                                            y: pc.position.y,
                                            sector: None,
                                            level: pc.layer,
                                        },
                                        is_pc: true,
                                        is_robin: pc.is_robin,
                                        is_vip: pc.is_vip,
                                    });
                            }
                            // Either way: don't add to
                            // enemy_sq_distances.
                            continue;
                        }
                        let dx = pc.position.x - eye.x;
                        let dy = (pc.position.y - eye.y)
                            * crate::position_interface::INVERSE_ASPECT_RATIO;
                        let sq_dist = (dx * dx + dy * dy) as i32;
                        tick_data
                            .enemy_sq_distances
                            .push((target_id.index(), sq_dist));
                        if sq_dist < tick_data.min_sq_enemy_distance {
                            tick_data.min_sq_enemy_distance = sq_dist;
                        }
                    }
                }

                // The count of enemies this soldier personally
                // detected (not shared by friends).
                tick_data.personally_visible_enemies = tick_data.enemy_sq_distances.len() as u16;

                // ── KillNearbySleepingEnemies scan ──────────────
                // Walk every enemy in the level and collect the
                // ones that are unconscious, not carried and pass
                // the NPC's 360°/LOS detection check.  This is the
                // final fallback used by `battle_decisions` when
                // there's literally nothing else left to do.
                //
                // Scoped to PCs here — unconscious enemy NPCs
                // would require iterating the opposing-camp
                // soldier list.  In practice only the player's
                // merry men can knock soldiers out, and the
                // battle path already prefers standing targets,
                // so the scan rarely matters.  Extending to
                // enemy-camp `soldier_snapshots` would duplicate
                // this loop with an additional camp filter.
                let view_radius_f = view_radius as f32;
                let sq_view_radius_kill = view_radius_f * view_radius_f;
                for pc in pc_snapshots {
                    if !pc.unconscious || pc.carried {
                        continue;
                    }
                    if pc.layer != layer {
                        continue;
                    }
                    // 360-degree detection: stretched-Y distance
                    // check against the real view radius, followed
                    // by a fast-grid LOS test against opaque
                    // obstacles.
                    let dx = pc.position.x - eye.x;
                    let dy =
                        (pc.position.y - eye.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
                    let sq_dist = dx * dx + dy * dy;
                    if sq_dist > sq_view_radius_kill {
                        continue;
                    }
                    if !ai_vision::los_clear_spatial(
                        eye,
                        pc.position,
                        layer,
                        sight_obstacles,
                        &self.world.fast_grid,
                    ) {
                        continue;
                    }
                    tick_data
                        .nearby_sleeping_enemies
                        .push(crate::ai::SleepingEnemyInfo {
                            handle: pc.id.index(),
                            position: crate::ai::Position {
                                x: pc.position.x,
                                y: pc.position.y,
                                sector: None,
                                level: pc.layer,
                            },
                            is_pc: true,
                            is_robin: pc.is_robin,
                            is_vip: pc.is_vip,
                        });
                }

                // Build us-list: nearby friendly soldiers.
                // 360° detection reduces to a distance check within
                // ~500 units.
                const US_LIST_SQ_RADIUS: f32 = 500.0 * 500.0;
                let my_company = enemy_ai.company_number;
                let my_pride = enemy_ai.soldier_profile_pride;
                enemy_ai.base.list_us.clear();
                enemy_ai.base.list_us.push(enemy_ai.base.me);
                tick_data.friends_lower_company = 0;
                tick_data.soldiers_lower_pride = false;
                // MakeBattlePredecisions: self contributes 100 + own pride.
                tick_data.us_battle_points = 100 + my_pride as u32;
                tick_data.has_officer_nearby = false;
                tick_data.simple_soldiers_near = false;
                tick_data.friends_nearer_to_enemy = 0;
                tick_data.visible_seeking_friends = 0;
                tick_data.friend_seek_clears_help_flag = false;

                // Also add visible PCs to us-list (they fight on our
                // side when the NPC is Royalist, but for Lacklandists
                // PCs are enemies — skip). For now, only add NPCs.
                for ss in soldier_snapshots {
                    if ss.id == npc_id || ss.camp != Camp::Lacklandists {
                        continue;
                    }
                    if !ss.able_to_fight {
                        continue;
                    }
                    if ss.layer != layer {
                        continue;
                    }
                    // Distance check
                    let fdx = ss.position.x - eye.x;
                    let fdy =
                        (ss.position.y - eye.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
                    let friend_sq_dist = fdx * fdx + fdy * fdy;
                    if friend_sq_dist > US_LIST_SQ_RADIUS {
                        continue;
                    }
                    // Only count soldiers in active states
                    match ss.ai_state {
                        AiState::Default
                        | AiState::Wondering
                        | AiState::Seeking
                        | AiState::Attacking => {}
                        _ => continue,
                    }
                    enemy_ai.base.list_us.push(ss.id.index());

                    // Company number tracking.
                    if my_company > ss.company_number
                        && (enemy_ai.base.current_substate
                            == crate::ai::Substate::AttackingReactiontime
                            || ss.ai_state == AiState::Attacking)
                    {
                        tick_data.friends_lower_company += 1;
                    }

                    // Pride tracking.
                    if my_pride > ss.pride {
                        tick_data.soldiers_lower_pride = true;
                    }

                    // Friend battle points.
                    tick_data.us_battle_points += 100 + ss.pride as u32;

                    // Simple soldiers near (for officer alert decision).
                    if ss.rank == crate::profiles::ProfileRank::Soldier {
                        tick_data.simple_soldiers_near = true;
                    }

                    // Officer nearby.
                    if ss.rank == crate::profiles::ProfileRank::Officer {
                        tick_data.has_officer_nearby = true;
                    }

                    // An attacking friend already in any swordfight /
                    // approach substate counts as occupying their
                    // primary target.  Otherwise, count the friend
                    // only if he is closer than us to our current
                    // primary target.
                    if ss.ai_state == AiState::Attacking && ss.primary_target != 0 {
                        if crate::ai_enemy::is_any_swordfight_substate(ss.ai_substate as u32) {
                            tick_data.friends_nearer_to_enemy += 1;
                        } else if let Some((_, _, best_score)) = best_target {
                            let to_enemy_sq = if let Some(pc) = pc_snapshots.first() {
                                let edx = ss.position.x - pc.position.x;
                                let edy = (ss.position.y - pc.position.y)
                                    * crate::position_interface::INVERSE_ASPECT_RATIO;
                                (edx * edx + edy * edy) as u32
                            } else {
                                u32::MAX
                            };
                            if to_enemy_sq < best_score {
                                tick_data.friends_nearer_to_enemy += 1;
                            }
                        }
                    }

                    // Visible friends in alert > Green that
                    // contribute to the seek-area point-factor
                    // multiplier.
                    if ss.alert_status != crate::ai::AlertLevel::Green {
                        tick_data.visible_seeking_friends += 1;

                        // If any friend is currently in a seek-area
                        // substate AND will look for help afterwards,
                        // clear our local LOOK_FOR_HELP flag so help
                        // isn't requested twice.
                        if ss.ai_substate.is_seek_area() && ss.seek_flag_look_for_help {
                            tick_data.friend_seek_clears_help_flag = true;
                        }
                    }

                    // Add attacking friends' primary targets to the
                    // them-list.
                    if ss.ai_state == AiState::Attacking
                        && ss.primary_target != 0
                        && !enemy_ai.list_them.contains(&ss.primary_target)
                    {
                        enemy_ai.list_them.push(ss.primary_target);
                    }
                }

                // Primary target multiplicity
                tick_data.primary_target_multiplicity.clear();
                for (&eid, &mult) in primary_target_multiplicity {
                    tick_data
                        .primary_target_multiplicity
                        .push((eid.index(), mult));
                }
                for &(attacker, target) in &self.ai.global.same_frame_target_claims {
                    if attacker == enemy_ai.base.me || target == 0 {
                        continue;
                    }
                    let Some(claimant) = soldier_snapshots
                        .iter()
                        .find(|ss| ss.id.index() == attacker)
                    else {
                        continue;
                    };
                    if claimant.camp != my_camp || !claimant.able_to_fight {
                        continue;
                    }
                    if target == enemy_ai.base.primary_target {
                        tick_data.friends_nearer_to_enemy =
                            tick_data.friends_nearer_to_enemy.saturating_add(1);
                    }
                    if let Some((_, mult)) = tick_data
                        .primary_target_multiplicity
                        .iter_mut()
                        .find(|(h, _)| *h == target)
                    {
                        *mult = mult.saturating_add(1);
                    } else {
                        tick_data.primary_target_multiplicity.push((target, 1));
                    }
                    if !enemy_ai.list_them.contains(&target) {
                        enemy_ai.list_them.push(target);
                    }
                }

                // ── Camp soldier snapshots for alert functions ──
                // Provides alert_officer / alert_soldiers with a view
                // of all same-camp soldiers (any distance).  The alert
                // functions do their own distance filtering.
                tick_data.camp_soldiers.clear();
                tick_data.camp_ko_money_fighters.clear();
                for (ko_id, ko_camp) in ko_money_fight_soldiers {
                    if *ko_id == npc_id || *ko_camp != my_camp {
                        continue;
                    }
                    tick_data.camp_ko_money_fighters.push(ko_id.index());
                }
                // is_detecting_360 is computed lazily by the AI consumer
                // (see EnemyAi::is_detecting_360_degrees) — eager LOS here
                // would fire O(N²) raycasts per AI tick.
                //
                // `is_detecting_cone` IS pre-computed (the cone-only
                // version's call surface — `MaybeOfficerSeesMeFighting`
                // — already gates on cheap rank/state filters first, so
                // the eager cost is bounded), against the brawler's
                // position so the per-call site reads a flag instead of
                // redoing the geometry per fighter pair.
                let me_in_building = viewer_building_sector.is_some();
                for ss in soldier_snapshots {
                    if ss.id == npc_id || ss.camp != my_camp {
                        continue;
                    }
                    let ss_position = crate::ai::Position {
                        x: ss.position.x,
                        y: ss.position.y,
                        sector: None,
                        level: ss.layer,
                    };
                    // Short-circuits inside `IsDetecting`: viewer
                    // blind / indoors / KO'd, or target indoors,
                    // → false.  Fold those into the cached value here.
                    let is_detecting_cone =
                        if ss.eye_blind || ss.in_building || !ss.able_to_fight || me_in_building {
                            false
                        } else {
                            crate::ai_vision::is_detecting_target(
                                ss.position,
                                ss.direction as i16,
                                (ss.view_direction[0], ss.view_direction[1]),
                                ss.real_half_aperture,
                                ss.view_radius,
                                me_pos_map,
                                layer,
                                sight_obstacles,
                                &self.world.fast_grid,
                            )
                        };
                    tick_data
                        .camp_soldiers
                        .push(crate::ai_enemy::CampSoldierInfo {
                            handle: ss.id.index(),
                            position: ss_position,
                            direction: ss.direction,
                            rank: ss.rank,
                            ai_state: ss.ai_state,
                            ai_substate: ss.ai_substate,
                            is_able_to_fight: ss.able_to_fight,
                            is_able_to_help: ss.able_to_help,
                            script_locked: ss.script_locked,
                            layer: ss.layer,
                            report_type: ss.report_type,
                            report_seek_position: ss.report_seek_position,
                            report_seen_bodies: ss.report_seen_bodies.clone(),
                            report_charly: ss.report_charly,
                            alert_soldiers_point: ss.alert_soldiers_point,
                            patrol_chief: ss.patrol_chief,
                            antagonist: ss.antagonist,
                            duty_flag: ss.duty_flag,
                            is_tower_guard: ss.is_tower_guard,
                            company_number: ss.company_number,
                            in_building: ss.in_building,
                            forecast_destination: ss.forecast_destination,
                            detectable_bodies: ss.detectable_bodies.clone(),
                            seek_position: ss.ai_seek_position,
                            current_task_priority: ss.current_task_priority,
                            minimal_task_priority: ss.minimal_task_priority,
                            view_direction: ss.view_direction,
                            view_radius: ss.view_radius,
                            real_half_aperture: ss.real_half_aperture,
                            eye_blind: ss.eye_blind,
                            is_detecting_cone,
                        });
                }

                // ── Fighter snapshots for swordfight tactics ─
                // The data the AI peeks at via entity pointers
                // (position, direction, weapon ranges, opponents),
                // built from the pre-computed pc/soldier snapshots
                // so we don't re-borrow the entity store.
                // Populated unconditionally so reaction-time paths
                // (FAST_OVERVIEW from EVENT_VIEW / EVENT_HEAR, which
                // fire before the NPC is swordfighting) can consult
                // it.  `FillListWithAllNearFighters` walks the
                // global fighter registry on every call, so the
                // snapshot needs to be available at all times.
                tick_data.nearby_fighters.clear();
                {
                    use crate::ai_enemy::FighterSnapshot;

                    // MAX_SWORDFIGHT_CONSIDERATION_RADIUS = 500.
                    // Uses Chebyshev (max-norm) distance for this check.
                    const SWORDFIGHT_RADIUS: f32 = 500.0;
                    let me_handle = enemy_ai.base.me;
                    let my_layer = layer;

                    // Self entry first.
                    if let Some(me_snap) =
                        soldier_snapshots.iter().find(|s| s.id.index() == me_handle)
                    {
                        tick_data.nearby_fighters.push(FighterSnapshot {
                            handle: me_handle,
                            position: crate::ai::Position {
                                x: me_snap.position.x,
                                y: me_snap.position.y,
                                sector: None,
                                level: my_layer,
                            },
                            direction: me_snap.direction,
                            is_friendly: true,
                            is_swordfighting: me_snap.is_swordfighting,
                            is_able_to_fight: me_snap.able_to_fight,
                            is_tied: me_snap.posture == Posture::Tied,
                            // Soldiers in `soldier_snapshots` are filtered to alive
                            // and conscious entries (snapshots.rs:L571), so these
                            // flags are constant `false` for any fighter sourced
                            // from there.
                            is_unconscious: false,
                            is_dead: false,
                            is_carried: false,
                            is_pc: false,
                            is_soldier: true,
                            rank: me_snap.rank,
                            primary_target: me_snap.primary_target,
                            principal_opponent: me_snap.principal_opponent,
                            opponent_handles: me_snap.opponent_handles.clone(),
                            number_of_opponents: me_snap
                                .opponent_handles
                                .len()
                                .min(u16::MAX as usize)
                                as u16,
                            sword_range_default: me_snap.sword_range_default,
                            sword_range_maximal: me_snap.sword_range_maximal,
                            sword_range_uber: me_snap.sword_range_uber,
                            fighting_ability: me_snap.fighting_ability,
                            has_formation: me_snap.has_formation,
                            is_vip: me_snap.is_vip,
                            is_tower_guard: me_snap.is_tower_guard,
                            soldier_profile_pride: me_snap.pride,
                            is_robin: false,
                            is_shield_bearer: me_snap.is_shield_bearer,
                            is_archer_unit: me_snap.is_archer_unit,
                            left_combat_neighbour: me_snap.left_combat_neighbour,
                            right_combat_neighbour: me_snap.right_combat_neighbour,
                            is_in_recovery_animation: me_snap.in_recovery,
                            in_sword_action_state: me_snap.action_state.is_sword(),
                            seek_position: crate::ai::Position {
                                x: me_snap.seek_position.x,
                                y: me_snap.seek_position.y,
                                sector: None,
                                level: my_layer,
                            },
                            archer_behind_me: me_snap.archer_behind_me,
                            ai_state: me_snap.ai_state,
                            shield_bearer_before_me: me_snap.shield_bearer_before_me,
                            current_substate: me_snap.ai_substate as u32,
                            hth_weapon_id: me_snap.hth_weapon_id,
                            action_state: me_snap.action_state,
                            shield_bearer_direction: me_snap.shield_bearer_direction,
                            shield_bearer_seek_position: crate::ai::Position {
                                x: me_snap.seek_position.x,
                                y: me_snap.seek_position.y,
                                sector: None,
                                level: my_layer,
                            },
                            bow_max_range: me_snap.bow_max_range,
                            elevation: me_snap.elevation,
                        });
                    }

                    // Friendly soldiers from the same-camp fighter
                    // registry (excluding self).
                    // ReconsiderSwordfightObservation rebuilds the
                    // us-list by scanning all nearby same-camp
                    // fighters every time; using the previous Rust
                    // `list_us` here made this snapshot stale and
                    // let multiple observers miss a friend already
                    // walking / running / charging the same target.
                    for ss in soldier_snapshots {
                        if ss.id.index() == me_handle || ss.camp != my_camp || !ss.able_to_fight {
                            continue;
                        }
                        if ss.layer != my_layer {
                            continue;
                        }
                        let dx = ss.position.x - eye.x;
                        let dy = (ss.position.y - eye.y)
                            * crate::position_interface::INVERSE_ASPECT_RATIO;
                        if dx.abs().max(dy.abs()) > SWORDFIGHT_RADIUS {
                            continue;
                        }
                        tick_data.nearby_fighters.push(FighterSnapshot {
                            handle: ss.id.index(),
                            position: crate::ai::Position {
                                x: ss.position.x,
                                y: ss.position.y,
                                sector: None,
                                level: ss.layer,
                            },
                            direction: ss.direction,
                            is_friendly: true,
                            is_swordfighting: ss.is_swordfighting,
                            is_able_to_fight: ss.able_to_fight,
                            is_tied: ss.posture == Posture::Tied,
                            is_unconscious: false,
                            is_dead: false,
                            is_carried: false,
                            is_pc: false,
                            is_soldier: true,
                            rank: ss.rank,
                            primary_target: ss.primary_target,
                            principal_opponent: ss.principal_opponent,
                            opponent_handles: ss.opponent_handles.clone(),
                            number_of_opponents: ss.opponent_handles.len().min(u16::MAX as usize)
                                as u16,
                            sword_range_default: ss.sword_range_default,
                            sword_range_maximal: ss.sword_range_maximal,
                            sword_range_uber: ss.sword_range_uber,
                            fighting_ability: ss.fighting_ability,
                            has_formation: ss.has_formation,
                            is_vip: ss.is_vip,
                            is_tower_guard: ss.is_tower_guard,
                            soldier_profile_pride: ss.pride,
                            is_robin: false,
                            is_shield_bearer: ss.is_shield_bearer,
                            is_archer_unit: ss.is_archer_unit,
                            left_combat_neighbour: ss.left_combat_neighbour,
                            right_combat_neighbour: ss.right_combat_neighbour,
                            is_in_recovery_animation: ss.in_recovery,
                            in_sword_action_state: ss.action_state.is_sword(),
                            seek_position: crate::ai::Position {
                                x: ss.seek_position.x,
                                y: ss.seek_position.y,
                                sector: None,
                                level: ss.layer,
                            },
                            archer_behind_me: ss.archer_behind_me,
                            ai_state: ss.ai_state,
                            shield_bearer_before_me: ss.shield_bearer_before_me,
                            current_substate: ss.ai_substate as u32,
                            hth_weapon_id: ss.hth_weapon_id,
                            action_state: ss.action_state,
                            shield_bearer_direction: ss.shield_bearer_direction,
                            shield_bearer_seek_position: crate::ai::Position {
                                x: ss.seek_position.x,
                                y: ss.seek_position.y,
                                sector: None,
                                level: ss.layer,
                            },
                            bow_max_range: ss.bow_max_range,
                            elevation: ss.elevation,
                        });
                    }

                    // Hostile PCs from the them-list.
                    for &enemy_handle in &enemy_ai.list_them {
                        let Some(pc) = pc_snapshots.iter().find(|p| p.id.index() == enemy_handle)
                        else {
                            continue;
                        };
                        if pc.layer != my_layer {
                            continue;
                        }
                        let dx = pc.position.x - eye.x;
                        let dy = (pc.position.y - eye.y)
                            * crate::position_interface::INVERSE_ASPECT_RATIO;
                        if dx.abs().max(dy.abs()) > SWORDFIGHT_RADIUS {
                            continue;
                        }
                        let number_of_opponents =
                            pc.opponent_handles.len().min(u16::MAX as usize) as u16;
                        tick_data.nearby_fighters.push(FighterSnapshot {
                            handle: enemy_handle,
                            position: crate::ai::Position {
                                x: pc.position.x,
                                y: pc.position.y,
                                sector: None,
                                level: pc.layer,
                            },
                            direction: pc.direction,
                            is_friendly: false,
                            is_swordfighting: pc.is_swordfighting,
                            is_able_to_fight: pc.able_to_fight,
                            is_tied: pc.posture == Posture::Tied,
                            is_unconscious: pc.unconscious,
                            // PCs in `pc_snapshots` are filtered to
                            // `life_points > 0` (snapshots.rs:L300).
                            is_dead: false,
                            is_carried: pc.carried,
                            is_pc: true,
                            is_soldier: false,
                            rank: crate::profiles::ProfileRank::None,
                            // Pull the PC's melee target from PcData.
                            primary_target: pc.melee_target.map(|id| id.index()).unwrap_or(0),
                            principal_opponent: pc.principal_opponent,
                            number_of_opponents,
                            opponent_handles: pc.opponent_handles.clone(),
                            sword_range_default: pc.sword_range_default,
                            sword_range_maximal: pc.sword_range_maximal,
                            sword_range_uber: pc.sword_range_uber,
                            fighting_ability: pc.fighting_ability,
                            has_formation: false,
                            is_vip: pc.is_vip,
                            is_tower_guard: false,
                            soldier_profile_pride: 0,
                            is_robin: pc.is_robin,
                            // PCs aren't shield bearers or archer units
                            // in the soldier-role sense (their combat
                            // behaviour is user-driven).
                            is_shield_bearer: false,
                            is_archer_unit: false,
                            left_combat_neighbour: 0,
                            right_combat_neighbour: 0,
                            is_in_recovery_animation: pc.in_recovery,
                            in_sword_action_state: pc.action_state.is_sword(),
                            seek_position: crate::ai::Position {
                                x: pc.position.x,
                                y: pc.position.y,
                                sector: None,
                                level: pc.layer,
                            },
                            // PCs never participate in archer↔shield pairing.
                            archer_behind_me: 0,
                            ai_state: AiState::default(),
                            shield_bearer_before_me: 0,
                            // PCs aren't AI-driven, so the substate
                            // concept doesn't apply — leave it 0.
                            current_substate: 0,
                            hth_weapon_id: pc.hth_weapon_id,
                            action_state: pc.action_state,
                            shield_bearer_direction: 0,
                            shield_bearer_seek_position: crate::ai::Position {
                                x: pc.position.x,
                                y: pc.position.y,
                                sector: None,
                                level: pc.layer,
                            },
                            bow_max_range: 0, // PCs don't use AI bow targeting
                            elevation: pc.ground_elevation,
                        });
                    }
                }
                think_tick_data = Some(tick_data);
            }

            // Accumulate the per-type detection suspects.
            let suspects = &mut soldier.npc.detection_suspects[enemy_idx];
            *suspects = suspects.saturating_add(sum_sharpness_new.min(u16::MAX as u32) as u16);

            // Running worst-detected-type (smallest enum value
            // wins).  We only drive Enemy detection here right now,
            // so the guard collapses to "promote from None / higher
            // to Enemy on any fresh sharpness this frame".  Body /
            // Object arms apply the same check when they are
            // ported.
            if sum_sharpness_new > 0
                && (soldier.npc.worst_detected_type as u32) > (DetectableType::Enemy as u32)
            {
                soldier.npc.worst_detected_type = DetectableType::Enemy;
            }

            // ── Pre-detection shadow event ────────────────────
            // Per-detectable edge-triggered EVENT_SEES_SHADOW on the
            // rising edge of
            //   shadow_is_seen = (sharpness > 0)
            //                 && suspects[type] >= SHADOW_DETECTION_THRESHOLD
            // No outer `instant_detection` / upper-bound guards.
            // Each detectable dispatches its own event on its own
            // rising edge, so no `break` after the first one.
            //
            // Skip PCs that are already guarded — once a soldier has
            // the PC in custody, no more shadow events fire for that
            // hero.  We still walk the latch update for non-guarded
            // PCs below.
            for det in soldier.npc.detectable_lists[enemy_idx].iter_mut() {
                let shadow_is_seen =
                    det.seen_now && *suspects as u32 >= ai_vision::SHADOW_DETECTION_THRESHOLD;
                let shadow_was_seen = det.shadow_seen_last_frame;
                det.shadow_seen_last_frame = shadow_is_seen;

                if shadow_is_seen
                    && !shadow_was_seen
                    && let Some(target_id) = det.element
                    && let Some(pc) = pc_snapshots.iter().find(|p| p.id == target_id)
                    && !pc.guarded
                {
                    // Queue EVENT_SEES_SHADOW for the
                    // post-detection pending_stimuli drain —
                    // see the EventHear site for rationale.
                    let shadow_pos = crate::ai::Position {
                        x: pc.position.x,
                        y: pc.position.y,
                        sector: None,
                        level: 0,
                    };
                    let stimulus = crate::ai::Stimulus::with_position(
                        crate::ai::StimulusType::EventSeesShadow,
                        shadow_pos,
                    );
                    if let Some(ai) = soldier.npc.ai_brain.base_mut() {
                        ai.pending_stimuli.push(stimulus);
                    }
                }
            }

            // Commit condition.
            let threshold_hit = *suspects as u32 >= ai_vision::DETECTION_SUSPECT_THRESHOLD;
            let instant_hit = instant_detection && sum_sharpness_new > 0;

            if threshold_hit || instant_hit {
                // Reset suspects on commit.
                *suspects = 0;

                if let Some((target_id, target_pos, _)) = best_target {
                    // ── Beggar detection routing ──────────────
                    // When the detected PC is disguised as a
                    // beggar, fire `EVENT_SEES_BEGGAR` instead of
                    // `EVENT_VIEW` — but only if the NPC's IQ
                    // >= CHECK_BEGGAR_MIN_IQ (30). Low-IQ soldiers
                    // ignore beggars entirely.
                    let target_posture = pc_snapshots
                        .iter()
                        .find(|pc| pc.id == target_id)
                        .map(|pc| pc.posture)
                        .unwrap_or_else(|| {
                            panic!(
                                "committed detection target {} is absent from the per-tick PC view",
                                target_id.index()
                            )
                        });

                    if target_posture == Posture::SimulatingBeggar {
                        // Original: `RHartificialmalignity.cpp::GetIQ` reads
                        // `uwIntelligence` and applies the enemy-IQ difficulty
                        // modifier. Fighting ability is a different capacity.
                        let intelligence = assets
                            .profile_manager
                            .get_soldier(soldier.soldier.soldier_profile_index)
                            .unwrap_or_else(|| {
                                panic!(
                                    "detecting soldier {} requires missing soldier profile {}",
                                    npc_id.index(),
                                    u32::from(soldier.soldier.soldier_profile_index)
                                )
                            })
                            .intelligence;
                        let npc_iq = crate::player_profile::DifficultyLevel::current()
                            .modify_capacity(
                                intelligence,
                                crate::player_profile::difficulty_params::EASY_ENEMY_IQ,
                                crate::player_profile::difficulty_params::HARD_ENEMY_IQ,
                                100,
                            ) as i32;
                        if npc_iq < crate::stealth::CHECK_BEGGAR_MIN_IQ {
                            // Too dumb to spot a beggar — skip.
                            return None;
                        }
                    }

                    // Only dispatch EVENT_VIEW on the rising edge of
                    // `seen_last_frame` for THIS detectable.  Without
                    // this gate, EVENT_VIEW re-fires every commit
                    // tick while the target stays visible, producing
                    // spurious state transitions.  MUST fall through
                    // to the latch-toggle block below so falling-edge
                    // detection still runs when we bypass dispatch —
                    // using `continue` here would skip the toggle.
                    //
                    // Rising-edge semantics: `seen_last_frame == false`
                    // at this point means the target was either never
                    // visible before or lost visibility at least one
                    // tick ago (the falling-edge arm clears the latch
                    // on any `!seen_now && seen_last_frame` commit or
                    // non-commit frame).  So a target re-detected
                    // after one tick of invisibility does fire
                    // EVENT_VIEW again — the `unwrap_or(true)` below
                    // also covers first-ever detection where the
                    // detectable entry was just inserted.
                    let rising_edge = soldier.npc.detectable_lists[enemy_idx]
                        .iter()
                        .find(|d| d.element == Some(target_id))
                        .map(|d| !d.seen_last_frame)
                        .unwrap_or(true);

                    let newly_alerted = soldier.npc.ai_state() != AiState::Attacking;
                    tracing::trace!(
                        npc = ?npc_id,
                        target = ?target_id,
                        ai_current_state = ?soldier.npc.ai_brain.base().map(|a| a.current_state),
                        ai_current_substate = ?soldier.npc.ai_brain.base().map(|a| a.current_substate),
                        newly_alerted,
                        rising_edge,
                        "detection commit check"
                    );
                    if rising_edge {
                        soldier.npc.alerted = true;

                        // Dispatch through the Think state machine
                        // instead of setting state directly.  For
                        // beggar targets, fire EventSeesBeggar so
                        // the AI enters the approach-and-identify
                        // substate chain instead of immediately
                        // attacking.
                        let stimulus_type = if target_posture == Posture::SimulatingBeggar {
                            crate::ai::StimulusType::EventSeesBeggar
                        } else {
                            crate::ai::StimulusType::EventView
                        };

                        if let Some(enemy_ai) = soldier.npc.ai_brain.enemy_mut() {
                            let stimulus =
                                crate::ai::Stimulus::with_human(stimulus_type, target_id.index());
                            enemy_ai.base.seek_position = crate::ai::Position {
                                x: target_pos.x,
                                y: target_pos.y,
                                sector: None,
                                level: 0,
                            };
                            // Retain until the soldier borrow is released.
                            // The outer per-NPC coordinator appends it after
                            // any predetection shadow stimulus, scans the
                            // remaining detectable buckets, then flushes the
                            // complete FIFO against a live boundary view.
                            think_stimulus = Some(stimulus);
                        }

                        // Always set music alert to Red on a fresh
                        // detection — the alert manager bumps it.
                        // Route via the soldier-side wrapper so the
                        // view field is updated too (Red is
                        // unaffected by the forced-attentive
                        // override but we go through the same path
                        // for consistency).
                        if let Some(enemy_ai) = soldier.npc.ai_brain.enemy_mut() {
                            enemy_ai.set_alert_status(crate::ai::AlertLevel::Red);
                        } else if let Some(ai) = soldier.npc.ai_brain.base_mut() {
                            ai.set_alert_status(crate::ai::AlertLevel::Red);
                        }
                        commit = Some((target_id, target_pos, newly_alerted));
                    }
                }
            } else if !any_seen_now
                && *suspects > 0
                && universal_frame.is_multiple_of(ai_vision::UNSUSPECT_FREQUENCY)
            {
                // Suspect cooldown when nothing visible.
                *suspects = suspects.saturating_sub(1);
            }

            // Recompute max-across-non-friend and reset worst-type
            // when nothing is suspect.  Runs after both the commit
            // (`*suspects = 0`) and decay arms so
            // `maximal_detection_suspect` always reflects the
            // post-frame value.  Only Enemy is maintained, so the
            // max reduces to that single entry.
            soldier.npc.maximal_detection_suspect = soldier.npc.detection_suspects[enemy_idx];
            if soldier.npc.maximal_detection_suspect == 0 {
                soldier.npc.worst_detected_type = DetectableType::None;
            }

            // Walk every detectable and edge-detect `seen_last_frame`.
            //   - Rising edge (detected && !latched) fires EVENT_VIEW
            //     (handled above for `best_target`; here we only
            //     toggle the latch so the question-mark emoticon
            //     accumulator stops climbing once the commit has
            //     fired).
            //   - Falling edge (!detected && latched) fires
            //     EVENT_OUTOFVIEW and clears the latch.
            // On commit frames both edges run; on non-commit frames
            // we still run the falling-edge check so NPCs react to
            // lost sight the instant it happens.
            let committed = threshold_hit || instant_hit;
            for det in soldier.npc.detectable_lists[enemy_idx].iter_mut() {
                let was_seen = det.seen_last_frame;
                let is_seen = det.seen_now;
                let falling_edge = !is_seen && was_seen;
                if falling_edge && let Some(target_id) = det.element {
                    out_of_view_dispatches.push((npc_id, target_id.index()));
                }
                if committed {
                    det.seen_last_frame = is_seen;
                } else if falling_edge {
                    det.seen_last_frame = false;
                }
                tracing::trace!(
                    npc = ?npc_id,
                    target = ?det.element,
                    committed,
                    threshold_hit,
                    instant_hit,
                    was_seen,
                    is_seen,
                    after_seen_last_frame = det.seen_last_frame,
                    "latch update"
                );
            }
        }

        if let Some((target_id, target_pos, newly_alerted)) = commit {
            transitions.push(Detection {
                enemy: npc_id,
                target: target_id,
                target_pos,
                newly_alerted,
            });
        }
        match (think_stimulus, think_tick_data) {
            (Some(stimulus), Some(tick_data)) => Some((stimulus, tick_data)),
            (None, _) => None,
            (Some(_), None) => {
                panic!("detection committed an enemy Think stimulus without per-tick enemy input")
            }
        }
    }

    /// P3b — royalist detection pass.
    ///
    /// Royalist soldiers detecting Lacklandist NPCs.  Implements the
    /// blip-reveal side effect and `HeyFolksLookThere` alert
    /// dispatch; full royalist combat AI (seeking, attacking) is
    /// deferred.
    pub(super) fn tick_enemy_ai_royalist_detection(
        &mut self,
        assets: &LevelAssets,
        world: &AiWorldView,
    ) {
        let universal_frame = self.control.frame_counter;
        let golden_eye = self.ai.global.golden_eye_mode;
        let is_forest_level = self.world.weather.is_forest_level;

        // Build target list from alive Lacklandist soldiers.
        let mut npc_targets: Vec<NpcTarget> = Vec::new();
        for s in &world.soldiers {
            if s.camp != Camp::Lacklandists {
                continue;
            }
            // `eye_z` is used as the detection point for
            // seen-by-pc / blip geometry; this is the target side of
            // the near-auto-visible check.
            let eye_z = s.ground_z + crate::stealth::detection_z_for_posture(s.posture, s.is_rider);
            npc_targets.push(NpcTarget {
                id: s.id,
                position: s.position,
                layer: s.layer,
                posture: s.posture,
                action_state: s.action_state,
                building_sector: s.building_sector,
                eye_z,
                direction: s.direction as i16,
                passing_door: s.passing_door,
                obstacle_idx: s.obstacle_idx,
            });
        }

        if npc_targets.is_empty() {
            return;
        }

        let mut to_reveal: Vec<EntityId> = Vec::new();
        let mut royalist_alert_calls: Vec<(EntityId, MapPoint)> = Vec::new();
        let royalist_ids = self.world.entities.npc_ids().collect::<Vec<_>>();

        for npc_id in royalist_ids {
            self.tick_enemy_ai_royalist_detection_for_npc(
                npc_id,
                assets,
                &npc_targets,
                universal_frame,
                golden_eye,
                is_forest_level,
                &mut to_reveal,
                &mut royalist_alert_calls,
            );
        }

        for target_id in to_reveal {
            if let Some(entity) = self.world.entities.get_mut(target_id)
                && entity.element_data().blipped
            {
                tracing::debug!(
                    entity = target_id.index(),
                    "reveal_blip: royalist detected blipped enemy"
                );
                entity.reveal_blip();
            }
        }

        // HeyFolksLookThere: alert nearby idle royalist soldiers
        // when a fresh detection commits.
        const ROYALIST_LOOK_THERE_RADIUS: f32 = 100.0;
        for (source_npc, target_pos) in royalist_alert_calls {
            self.hey_folks_look_there(source_npc, target_pos, ROYALIST_LOOK_THERE_RADIUS);
        }
    }

    /// P3b inner — per-NPC body of [`Self::tick_enemy_ai_royalist_detection`].
    /// Carries the per-NPC tracing span.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    #[allow(clippy::too_many_arguments)]
    fn tick_enemy_ai_royalist_detection_for_npc(
        &mut self,
        npc_id: EntityId,
        assets: &LevelAssets,
        npc_targets: &[NpcTarget],
        universal_frame: u32,
        golden_eye: bool,
        is_forest_level: bool,
        to_reveal: &mut Vec<EntityId>,
        royalist_alert_calls: &mut Vec<(EntityId, MapPoint)>,
    ) {
        // -- Read royalist soldier viewer state --
        let viewer = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let Some(viewer) = SoldierSightContext::from_viewer(entity, Camp::Royalists) else {
                return;
            };
            viewer
        };
        if viewer.ai_locked {
            return;
        }
        let eye = viewer.eye;
        let eye_z = viewer.eye_z;
        let dir = viewer.dir;
        let layer = viewer.layer;
        let view_radius = viewer.view_radius;
        let eye_status = viewer.eye_status;
        let current_state = viewer.current_state;
        let view_forward = viewer.view_forward;
        let real_half_aperture = viewer.real_half_aperture;
        let npc_posture = viewer.posture;
        let entity_sector = viewer.sector;
        let is_rider_npc = viewer.is_rider;
        let alert_status = viewer.alert_status;

        let viewer_building_sector = self.entity_building_sector(entity_sector);

        // Effective view radius accounting for eye height and
        // night/fog light modulation.
        let is_night_or_fog = matches!(
            self.world.weather.ambiance,
            crate::engine::types::Ambiance::Night | crate::engine::types::Ambiance::Fog
        );
        let effective_view_radius_ground = ai_vision::compute_view_radius(
            eye,
            eye_z,
            view_radius,
            view_forward,
            real_half_aperture,
            is_night_or_fog,
            &self.world.fast_grid.level,
            self.sight_obstacles(assets),
            None,
        );
        // Per-target obstacle-aware re-call.  Targets standing on a
        // roof / ledge / balcony get an obstacle-aware radius;
        // ground targets reuse the cached ground value.
        let per_target_view_radius: std::collections::HashMap<EntityId, f32> = {
            let obstacles = self.sight_obstacles(assets);
            npc_targets
                .iter()
                .filter_map(|t| {
                    let h = t.obstacle_idx?;
                    let obs = obstacles.get(usize::from(h))?;
                    let r = ai_vision::compute_view_radius(
                        eye,
                        eye_z,
                        view_radius,
                        view_forward,
                        real_half_aperture,
                        is_night_or_fog,
                        &self.world.fast_grid.level,
                        obstacles,
                        Some(obs),
                    );
                    Some((t.id, r))
                })
                .collect()
        };
        // Per-NPC frame-counter phase offset — EntityId stands in
        // for the creation counter since slots are monotonic and
        // never reused.
        let modified_frame = universal_frame.wrapping_add(npc_id.index());
        // Royalists detecting enemy NPCs use
        // `DETECTION_FREQUENCY_ENEMY_NPC` (16), not the PC variant
        // (2).  `refresh_always` is true when eye status is
        // Stare / Follow or when alert_status is anything other than
        // Green — that bypasses the per-NPC frequency gate so
        // staring / on-alert royalists refresh visibility every
        // tick instead of only on the gate-open frame.
        let refresh_always = matches!(
            eye_status,
            crate::element::EyeStatus::Stare | crate::element::EyeStatus::Follow
        ) || !matches!(alert_status, crate::ai::AlertLevel::Green);
        let gate_open = refresh_always
            || modified_frame.is_multiple_of(ai_vision::DETECTION_FREQUENCY_ENEMY_NPC);
        // InstantDetection for Royalist enemies is always true —
        // royalist soldiers at peace commit a sighting immediately
        // rather than waiting for the `suspects >= 1000` slow path.
        let instant_detection = true;
        let _ = current_state; // (state no longer gates Royalist detection)

        // -- Mutating pass: detectable list + suspects --
        let mut commit_target: Option<(EntityId, MapPoint)> = None;
        {
            // Build the obstacle view from individual fields
            // so the borrow checker can disjoint-split it
            // from the mut borrows of `ai_global` / `entities`.
            let sight_obstacles = crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.static_sight_obstacles.as_slice(),
                dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                static_active: &self.world.static_sight_obstacle_active,
            };
            // Split-borrow ai_global (kept live so the
            // royalist detection below still compiles —
            // the now-deferred EVENT_VIEW push doesn't
            // need it).
            let _ai_global = &mut self.ai.global;
            let Some(Entity::Soldier(soldier)) = self.world.entities.get_mut(npc_id) else {
                return;
            };

            let enemy_idx = DetectableType::Enemy as usize;
            let detectables = &mut soldier.npc.detectable_lists[enemy_idx];

            // Lazy-populate with Lacklandist NPC targets.
            for target in npc_targets.iter() {
                if !detectables.iter().any(|d| d.element == Some(target.id)) {
                    detectables.push(Detectable {
                        element: Some(target.id),
                        detectable_type: DetectableType::Enemy,
                        ..Default::default()
                    });
                }
            }
            detectables.retain(|d| {
                d.element
                    .is_some_and(|id| npc_targets.iter().any(|t| t.id == id))
            });

            let mut sum_sharpness_new: u32 = 0;
            let mut any_seen_now = false;
            let mut best_target: Option<(EntityId, MapPoint)> = None;

            for det in detectables.iter_mut() {
                let Some(target_id) = det.element else {
                    continue;
                };
                let Some(target) = npc_targets.iter().find(|t| t.id == target_id) else {
                    continue;
                };

                if target.layer != layer {
                    det.seen_now = false;
                    det.last_visibility = 0.0;
                    continue;
                }

                let visibility_raw = if gate_open {
                    let viewer_in_building = viewer_building_sector.is_some();
                    let target_in_same_building =
                        viewer_in_building && viewer_building_sector == target.building_sector;
                    // Per-target effective view radius.
                    let effective_view_radius = per_target_view_radius
                        .get(&target_id)
                        .copied()
                        .unwrap_or(effective_view_radius_ground);
                    let q = ai_vision::VisibilityQuery {
                        viewer: eye,
                        viewer_direction: dir,
                        view_forward,
                        view_radius,
                        viewer_eye_status: eye_status,
                        real_half_aperture,
                        viewer_in_building,
                        target_in_same_building,
                        // 180° merry-man-forest view: royalist
                        // non-riders on forest levels get flat
                        // 180° detection instead of a narrow cone.
                        forest_180_degree_view: is_forest_level && !is_rider_npc,
                        golden_eye_mode: golden_eye,
                        effective_view_radius,
                        // Lacklandist targets in npc_targets
                        // are filtered to active/alive above,
                        // so this reduces to "not in a building".
                        target_is_active_and_outside_building: target.building_sector.is_none(),
                        target: crate::stealth::detection_point_xy(
                            target.position,
                            target.posture,
                            target.direction,
                        ),
                        target_posture: target.posture,
                        target_action_state: target.action_state,
                        target_is_pc: false,
                        viewer_eye_z: eye_z,
                        target_eye_z: target.eye_z,
                        sight_obstacles,
                        fast_grid: &self.world.fast_grid,
                        layer,
                        // NpcTarget list filters out
                        // unconscious soldiers at build time
                        // (see `ok` check above), so
                        // targets reaching here are always
                        // conscious.
                        target_unconscious: false,
                        target_passing_door: target.passing_door,
                    };
                    ai_vision::compute_visibility(&q)
                } else {
                    0.0
                };

                let visibility = if gate_open {
                    ai_vision::DETECTION_FREQUENCY_ENEMY_NPC as f32 * visibility_raw
                } else {
                    // Closed-gate frame — reuse cached
                    // post-multiplied value from the prior refresh.
                    det.last_visibility
                };
                let view_speed = if npc_posture == Posture::LeaningOut {
                    ai_vision::LOOK_DOWN_BASE_VIEW_SPEED
                } else {
                    ai_vision::BASE_VIEW_SPEED
                };
                let sharpness = (view_speed as f32 * visibility) as u32;
                let is_visible = sharpness > 0;

                if is_visible && !det.seen_last_frame {
                    sum_sharpness_new = sum_sharpness_new.saturating_add(sharpness);
                }
                if is_visible {
                    any_seen_now = true;
                    if best_target.is_none() {
                        best_target = Some((target_id, target.position));
                    }
                }

                det.seen_now = is_visible;
                // Store the post-frequency-multiplied value;
                // closed-gate frames re-read this above.
                if gate_open {
                    det.last_visibility = visibility;
                }
            }

            // Accumulate suspects.
            let suspects = &mut soldier.npc.detection_suspects[enemy_idx];
            *suspects = suspects.saturating_add(sum_sharpness_new.min(u16::MAX as u32) as u16);

            // Running worst-detected-type (see the twin site for the
            // single-type rationale).
            if sum_sharpness_new > 0
                && (soldier.npc.worst_detected_type as u32) > (DetectableType::Enemy as u32)
            {
                soldier.npc.worst_detected_type = DetectableType::Enemy;
            }

            // Commit condition.
            let threshold_hit = *suspects as u32 >= ai_vision::DETECTION_SUSPECT_THRESHOLD;
            let instant_hit = instant_detection && sum_sharpness_new > 0;

            if threshold_hit || instant_hit {
                *suspects = 0;

                if let Some((target_id, target_pos)) = best_target {
                    // ── Dispatch EVENT_VIEW to royalist AI ──
                    // On commit, fire EVENT_VIEW so the royalist AI
                    // can react (attack, chase, etc.).
                    soldier.npc.alerted = true;

                    if let Some(enemy_ai) = soldier.npc.ai_brain.enemy_mut() {
                        enemy_ai.base.seek_position = crate::ai::Position {
                            x: target_pos.x,
                            y: target_pos.y,
                            sector: None,
                            level: 0,
                        };
                        let stimulus = crate::ai::Stimulus::with_human(
                            crate::ai::StimulusType::EventView,
                            target_id.index(),
                        );
                        // Queue for post-detection drain — see
                        // the EventHear site for rationale.
                        enemy_ai.base.pending_stimuli.push(stimulus);
                    }

                    commit_target = Some((target_id, target_pos));
                }
            } else if !any_seen_now
                && *suspects > 0
                && universal_frame.is_multiple_of(ai_vision::UNSUSPECT_FREQUENCY)
            {
                *suspects = suspects.saturating_sub(1);
            }

            // Post-frame max + worst-type reset.  See the twin site
            // above for rationale.
            soldier.npc.maximal_detection_suspect = soldier.npc.detection_suspects[enemy_idx];
            if soldier.npc.maximal_detection_suspect == 0 {
                soldier.npc.worst_detected_type = DetectableType::None;
            }

            // Maintain `seen_last_frame` so the sharpness
            // accumulator above keeps firing every frame the target
            // stays visible, instead of once per visibility edge.
            // See the matching block in the enemy→PC detection path
            // above.
            let committed = threshold_hit || instant_hit;
            for det in soldier.npc.detectable_lists[enemy_idx].iter_mut() {
                if committed {
                    det.seen_last_frame = det.seen_now;
                } else if !det.seen_now && det.seen_last_frame {
                    det.seen_last_frame = false;
                }
            }
        }

        // Royalist detects blipped NPC → reveal.
        if let Some((target_id, target_pos)) = commit_target {
            to_reveal.push(target_id);
            royalist_alert_calls.push((npc_id, target_pos));
        }
    }

    // ── P3c. Per-NPC non-Enemy detection (Body / Object /
    //         Friend / MissedFriend / Beggar) ────────────────────
    //
    // Per-`type` outer arms of `RefreshDetection` for every
    // detectable bucket except `DETECTABLE_ENEMY` (which is handled
    // by the existing Lacklandist→PC + Royalist→NPC passes earlier
    // in the tick).  Runs after those passes settle so each NPC's
    // `detection_suspects[Enemy]` is finalized before this pass
    // contributes its own per-type entries to
    // `maximal_detection_suspect` / `worst_detected_type`.
    //
    // What lands here per kind (all Lacklandist-camp NPCs only —
    // the Royalist arm returns 0 for every non-Enemy type, so the
    // camp gate below is parity, not a deferral):
    //  - Body: gates on `IgnoreBodies()` + `viewer_in_building`;
    //    visibility = `BODY_DETECTION_FACTOR * DETECTION_FREQUENCY_BODY
    //    * compute_visibility(body_as_human)`; `InstantDetection`
    //    rule `!matches!(state, Sleeping|Default|Wondering)`;
    //    rising-edge `EventSeesBody` + drop-on-commit; participates in
    //    `maximal_detection_suspect` (`type < FRIEND`);
    //    HandlePredetection shadow events for PC-typed bodies (the
    //    `IsPC()` check effectively restricts shadow dispatch to
    //    PC bodies).
    //  - Object: gates on `viewer_in_building`; visibility =
    //    `DETECTION_FREQUENCY_OBJECT * compute_object_visibility(...)`;
    //    `InstantDetection` rule
    //    `!matches!(state, Sleeping|Default)` (note: Wondering is
    //    instant for Objects, unlike Body/Enemy);
    //    rising-edge `EventSeesObject` + drop-on-commit; participates
    //    in `maximal_detection_suspect`; inline `CleanUpDetectables`
    //    drops `!active` entries.  No shadow events —
    //    HandlePredetection's `IsPC()` gate skips Objects
    //    unconditionally.
    //  - Friend: gate `!IsAbleToHelp() || viewer_in_building`;
    //    visibility = `DETECTION_FREQUENCY_FRIEND *
    //    compute_visibility(human)`; `InstantDetection` always
    //    true; rising-edge `EventSeesSoldier` + drop-on-commit; does
    //    NOT contribute to `maximal_detection_suspect`
    //    (`type < FRIEND`).  No shadow events.
    //  - MissedFriend: gate `IsDead() || IsUnconscious() ||
    //    viewer_in_building`; visibility =
    //    `DETECTION_FREQUENCY_MISSED_FRIEND *
    //    compute_visibility(human)`; `InstantDetection` always
    //    true; rising-edge `EventSeesCharly` + drop-on-commit; does
    //    NOT contribute to `maximal_detection_suspect`.
    //  - Beggar: gate `IsDead() || IsUnconscious() ||
    //    viewer_in_building`; visibility =
    //    `DETECTION_FREQUENCY_BEGGAR * compute_visibility(human)`;
    //    `InstantDetection` always true; rising-edge
    //    `EventSeesBeggar` + drop-on-commit; does NOT contribute to
    //    `maximal_detection_suspect`.  Inline `CleanUpDetectables`
    //    drops entries whose target is no longer
    //    `IsTrueOrFalseBeggar()`.
    /// Per-NPC body of the non-Enemy portion of `RefreshDetection`.
    /// One full iteration of the per-type loop body for
    /// `type ∈ {Body, Object, Friend, MissedFriend, Beggar}`.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    #[allow(clippy::too_many_arguments)]
    fn tick_enemy_ai_refresh_per_type_for_npc(
        &mut self,
        npc_id: EntityId,
        assets: &LevelAssets,
        human_targets: &std::collections::HashMap<EntityId, HumanTarget>,
        object_targets: &std::collections::HashMap<EntityId, ObjectTarget>,
        universal_frame: u32,
        golden_eye: bool,
    ) {
        use crate::ai::AiState;

        // -- Read NPC view-state in a scoped read borrow --
        let viewer = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            // RefreshDetection runs the per-type loop for both
            // camps.  Restrict to Lacklandists for now — the audit
            // notes Royalist body/object reactions have no consumer
            // wired in the Rust AI layer yet, and exposing the loop
            // there would create dead stimuli with no handlers.
            let Some(viewer) = SoldierSightContext::from_viewer(entity, Camp::Lacklandists) else {
                return;
            };
            viewer
        };
        if viewer.ai_locked {
            return;
        }
        let eye = viewer.eye;
        let eye_z = viewer.eye_z;
        let dir = viewer.dir;
        let layer = viewer.layer;
        let view_radius = viewer.view_radius;
        let eye_status = viewer.eye_status;
        let current_state = viewer.current_state;
        let view_forward = viewer.view_forward;
        let real_half_aperture = viewer.real_half_aperture;
        let npc_posture = viewer.posture;
        let current_substate = viewer.current_substate;
        let alert_status = viewer.alert_status;
        let ignore_bodies = viewer.ignore_bodies;
        let _ = (
            current_substate,
            viewer.blipped,
            viewer.camp,
            viewer.action_state,
        ); // suppress unused-warning when individual gates not consulted

        let viewer_building_sector = self.entity_building_sector(viewer.sector);
        let viewer_in_building = viewer_building_sector.is_some();

        let is_night_or_fog = matches!(
            self.world.weather.ambiance,
            crate::engine::types::Ambiance::Night | crate::engine::types::Ambiance::Fog
        );
        let effective_view_radius_ground = ai_vision::compute_view_radius(
            eye,
            eye_z,
            view_radius,
            view_forward,
            real_half_aperture,
            is_night_or_fog,
            &self.world.fast_grid.level,
            self.sight_obstacles(assets),
            None,
        );
        // Per-target obstacle-aware re-call.  Pre-computed across
        // the union of human targets so the Body / Friend /
        // MissedFriend / Beggar passes (each going through
        // `run_human_detectable_pass`) all share the same map.
        // Targets without a projection obstacle are absent and fall
        // back to `effective_view_radius_ground` inside the helper.
        let per_target_view_radius: std::collections::HashMap<EntityId, f32> = {
            let obstacles = self.sight_obstacles(assets);
            human_targets
                .iter()
                .filter_map(|(id, t)| {
                    let h = t.obstacle_idx?;
                    let obs = obstacles.get(usize::from(h))?;
                    let r = ai_vision::compute_view_radius(
                        eye,
                        eye_z,
                        view_radius,
                        view_forward,
                        real_half_aperture,
                        is_night_or_fog,
                        &self.world.fast_grid.level,
                        obstacles,
                        Some(obs),
                    );
                    Some((*id, r))
                })
                .collect()
        };
        // Per-NPC frame phase offset.
        let modified_frame = universal_frame.wrapping_add(npc_id.index());

        // refresh-always gate: Stare / Follow eye status and alert
        // levels above Green force the per-type frequency gate open
        // so visibility refreshes every tick.
        let refresh_always = matches!(
            eye_status,
            crate::element::EyeStatus::Stare | crate::element::EyeStatus::Follow
        ) || !matches!(alert_status, crate::ai::AlertLevel::Green);

        const BODY_DETECTION_FACTOR: f32 = 3.0;

        // Reusable view-speed for `sharpness = view_speed * visibility`.
        let view_speed = if npc_posture == Posture::LeaningOut {
            ai_vision::LOOK_DOWN_BASE_VIEW_SPEED
        } else {
            ai_vision::BASE_VIEW_SPEED
        };

        // Pull the obstacle view + soldier mut borrow for the rest of
        // the function.  The body/object detectable lists, suspect
        // counters, and pending_stimuli all live under `soldier.npc`,
        // so we keep one mut-borrow scope spanning both passes.
        let sight_obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        let _ai_global = &mut self.ai.global;
        let Some(Entity::Soldier(soldier)) = self.world.entities.get_mut(npc_id) else {
            return;
        };

        // ── BODY pass ───────────────────────────────────────
        Self::run_human_detectable_pass(
            soldier,
            npc_id,
            DetectableType::Body,
            ai_vision::DETECTION_FREQUENCY_BODY,
            BODY_DETECTION_FACTOR * ai_vision::DETECTION_FREQUENCY_BODY as f32,
            // InstantDetection for Body (Lacklandists):
            // `!matches!(state, Sleeping|Default|Wondering)`.
            !matches!(
                current_state,
                AiState::Sleeping | AiState::Default | AiState::Wondering
            ),
            crate::ai::StimulusType::EventSeesBody,
            // Body counts toward `maximal_detection_suspect`
            // (`type < FRIEND`).
            true,
            // Body fires HandlePredetection shadow events for PC
            // bodies (the `IsPC()` gate).
            true,
            // Body's per-pass extra gate combines IgnoreBodies +
            // viewer_in_building.
            ignore_bodies,
            human_targets,
            // Per-target pre-filter — Body has no extra check beyond
            // the layer match enforced by the helper.
            |_t| true,
            ViewContext {
                eye,
                eye_z,
                dir,
                layer,
                view_forward,
                view_radius,
                real_half_aperture,
                viewer_in_building,
                viewer_building_sector,
                effective_view_radius_ground,
                per_target_view_radius: &per_target_view_radius,
                eye_status,
                view_speed,
                refresh_always,
                modified_frame,
                universal_frame,
                golden_eye,
                sight_obstacles: &sight_obstacles,
                fast_grid: &self.world.fast_grid,
            },
        );

        // ── OBJECT pass ─────────────────────────────────────
        // Original detectable enum order is Enemy, Body, Object,
        // Friend, MissedFriend, Beggar. Keep stimulus queue order aligned
        // with that scan order before the per-NPC FIFO Think drain.
        Self::run_object_detectable_pass(
            soldier,
            npc_id,
            ai_vision::DETECTION_FREQUENCY_OBJECT,
            // InstantDetection for OBJECT (Lacklandists) is
            // `!matches!(state, Sleeping|Default)` — Wondering IS
            // instant for Objects.
            !matches!(current_state, AiState::Sleeping | AiState::Default),
            object_targets,
            ViewContext {
                eye,
                eye_z,
                dir,
                layer,
                view_forward,
                view_radius,
                real_half_aperture,
                viewer_in_building,
                viewer_building_sector,
                effective_view_radius_ground,
                per_target_view_radius: &per_target_view_radius,
                eye_status,
                view_speed,
                refresh_always,
                modified_frame,
                universal_frame,
                golden_eye,
                sight_obstacles: &sight_obstacles,
                fast_grid: &self.world.fast_grid,
            },
        );

        // ── FRIEND pass ─────────────────────────────────────
        Self::run_human_detectable_pass(
            soldier,
            npc_id,
            DetectableType::Friend,
            ai_vision::DETECTION_FREQUENCY_FRIEND,
            ai_vision::DETECTION_FREQUENCY_FRIEND as f32,
            // InstantDetection for Friend always true.
            true,
            crate::ai::StimulusType::EventSeesSoldier,
            // `type < FRIEND` — Friend itself does NOT contribute to
            // `maximal_detection_suspect`.
            false,
            // No shadow events (early return for Friend).
            false,
            // Per-pass extra gate: Friend uses viewer_in_building
            // alone, no IgnoreBodies override.
            false,
            human_targets,
            // Per-target pre-filter: target must `IsAbleToHelp()`.
            |t| t.able_to_help,
            ViewContext {
                eye,
                eye_z,
                dir,
                layer,
                view_forward,
                view_radius,
                real_half_aperture,
                viewer_in_building,
                viewer_building_sector,
                effective_view_radius_ground,
                per_target_view_radius: &per_target_view_radius,
                eye_status,
                view_speed,
                refresh_always,
                modified_frame,
                universal_frame,
                golden_eye,
                sight_obstacles: &sight_obstacles,
                fast_grid: &self.world.fast_grid,
            },
        );

        // ── MISSED_FRIEND pass ──────────────────────────────
        Self::run_human_detectable_pass(
            soldier,
            npc_id,
            DetectableType::MissedFriend,
            ai_vision::DETECTION_FREQUENCY_MISSED_FRIEND,
            ai_vision::DETECTION_FREQUENCY_MISSED_FRIEND as f32,
            // Always-true InstantDetection.
            true,
            crate::ai::StimulusType::EventSeesCharly,
            // Does not contribute to `maximal_detection_suspect`.
            false,
            // No shadow events (early return for MissedFriend).
            false,
            false,
            human_targets,
            // Per-target pre-filter: skip dead / unconscious targets.
            |t| !t.unconscious,
            ViewContext {
                eye,
                eye_z,
                dir,
                layer,
                view_forward,
                view_radius,
                real_half_aperture,
                viewer_in_building,
                viewer_building_sector,
                effective_view_radius_ground,
                per_target_view_radius: &per_target_view_radius,
                eye_status,
                view_speed,
                refresh_always,
                modified_frame,
                universal_frame,
                golden_eye,
                sight_obstacles: &sight_obstacles,
                fast_grid: &self.world.fast_grid,
            },
        );

        // ── BEGGAR pass ─────────────────────────────────────
        // CleanUpDetectables for BEGGAR drops entries whose target
        // is no longer `IsTrueOrFalseBeggar()`.  Run that prune
        // ahead of the visibility loop so the helper doesn't
        // compute visibility for stale entries.
        {
            let beggar_idx = DetectableType::Beggar as usize;
            soldier.npc.detectable_lists[beggar_idx].retain(|det| {
                let Some(target_id) = det.element else {
                    return false;
                };
                human_targets
                    .get(&target_id)
                    .map(|t| t.is_true_or_false_beggar)
                    .unwrap_or(false)
            });
        }
        Self::run_human_detectable_pass(
            soldier,
            npc_id,
            DetectableType::Beggar,
            ai_vision::DETECTION_FREQUENCY_BEGGAR,
            ai_vision::DETECTION_FREQUENCY_BEGGAR as f32,
            true,
            crate::ai::StimulusType::EventSeesBeggar,
            false,
            false,
            false,
            human_targets,
            // Per-target pre-filter: skip dead / unconscious targets.
            |t| !t.unconscious,
            ViewContext {
                eye,
                eye_z,
                dir,
                layer,
                view_forward,
                view_radius,
                real_half_aperture,
                viewer_in_building,
                viewer_building_sector,
                effective_view_radius_ground,
                per_target_view_radius: &per_target_view_radius,
                eye_status,
                view_speed,
                refresh_always,
                modified_frame,
                universal_frame,
                golden_eye,
                sight_obstacles: &sight_obstacles,
                fast_grid: &self.world.fast_grid,
            },
        );
    }

    /// Per-NPC per-type detection helper for the four
    /// human-targeting buckets — `Body`, `Friend`, `MissedFriend`,
    /// `Beggar`.  One full iteration of the per-type loop body:
    /// per-detectable visibility (`compute_visibility` scaled by
    /// `factor`), suspect accumulation, threshold-or-instant commit,
    /// rising-edge `event_type` dispatch with drop-on-commit removal,
    /// suspect cooldown, `maximal_detection_suspect` /
    /// `worst_detected_type` contribution.
    ///
    /// `extra_gate_blocks_visibility` is the per-kind boolean
    /// short-circuit checked before computing visibility (e.g.
    /// `IgnoreBodies()` for Body).  `viewer_in_building` is always
    /// applied on top.  `target_pre_filter` runs per target
    /// (`IsAbleToHelp()` for Friend, `!IsDead && !IsUnconscious`
    /// for MissedFriend / Beggar — Body has no per-target filter
    /// beyond the layer match).
    ///
    /// `fire_shadow_for_pc_targets`: when true, runs
    /// `HandlePredetection` inline — only Body satisfies this
    /// (the Enemy arm has its own dedicated shadow-event block
    /// earlier in the tick; FRIEND / MISSED_FRIEND / BEGGAR are
    /// skipped at the early-return; OBJECT is skipped via the
    /// `IsPC()` gate).
    ///
    /// `contribute_to_maximal`: matches `type < FRIEND` — only Body
    /// and Object contribute to `maximal_detection_suspect`; the
    /// three FRIEND-and-after buckets do not.
    #[allow(clippy::too_many_arguments)]
    fn run_human_detectable_pass<F>(
        soldier: &mut crate::element::ActorSoldier,
        npc_id: EntityId,
        kind: DetectableType,
        frequency: u32,
        factor: f32,
        instant_detection: bool,
        event_type: crate::ai::StimulusType,
        contribute_to_maximal: bool,
        fire_shadow_for_pc_targets: bool,
        extra_gate_blocks_visibility: bool,
        targets: &std::collections::HashMap<EntityId, HumanTarget>,
        target_pre_filter: F,
        ctx: ViewContext<'_>,
    ) where
        F: Fn(&HumanTarget) -> bool,
    {
        let kind_idx = kind as usize;
        let gate_open = ctx.refresh_always || ctx.modified_frame.is_multiple_of(frequency);

        let mut sum_of_sharpnesses: u32 = 0;

        // (1) Per-detectable visibility pass.
        for det in soldier.npc.detectable_lists[kind_idx].iter_mut() {
            let Some(target_id) = det.element else {
                det.seen_now = false;
                det.last_visibility = 0.0;
                continue;
            };
            let Some(target) = targets.get(&target_id) else {
                det.seen_now = false;
                det.last_visibility = 0.0;
                continue;
            };
            if target.layer != ctx.layer {
                det.seen_now = false;
                det.last_visibility = 0.0;
                continue;
            }

            let visibility: f32 = if extra_gate_blocks_visibility
                || ctx.viewer_in_building
                || !target_pre_filter(target)
            {
                0.0
            } else if gate_open {
                let target_in_same_building = ctx.viewer_in_building
                    && ctx.viewer_building_sector == target.building_sector
                    && !target.unconscious;
                // Per-target effective view radius.  Targets
                // without an obstacle reuse the once-per-viewer
                // ground value.
                let effective_view_radius = ctx
                    .per_target_view_radius
                    .get(&target_id)
                    .copied()
                    .unwrap_or(ctx.effective_view_radius_ground);
                let q = ai_vision::VisibilityQuery {
                    viewer: ctx.eye,
                    viewer_direction: ctx.dir,
                    view_forward: ctx.view_forward,
                    view_radius: ctx.view_radius,
                    viewer_eye_status: ctx.eye_status,
                    real_half_aperture: ctx.real_half_aperture,
                    viewer_in_building: ctx.viewer_in_building,
                    target_in_same_building,
                    forest_180_degree_view: false,
                    golden_eye_mode: ctx.golden_eye,
                    effective_view_radius,
                    target_is_active_and_outside_building: target.active
                        && target.building_sector.is_none(),
                    target: crate::stealth::detection_point_xy(
                        target.position,
                        target.posture,
                        target.direction,
                    ),
                    target_posture: target.posture,
                    target_action_state: target.action_state,
                    target_is_pc: target.is_pc,
                    viewer_eye_z: ctx.eye_z,
                    target_eye_z: target.eye_z,
                    sight_obstacles: *ctx.sight_obstacles,
                    fast_grid: ctx.fast_grid,
                    layer: ctx.layer,
                    target_unconscious: target.unconscious,
                    target_passing_door: target.passing_door,
                };
                factor * ai_vision::compute_visibility(&q)
            } else {
                det.last_visibility
            };

            let sharpness = (ctx.view_speed as f32 * visibility) as u32;
            let is_visible = sharpness > 0;

            if !det.seen_last_frame {
                sum_of_sharpnesses = sum_of_sharpnesses.saturating_add(sharpness);
            }

            det.seen_now = is_visible;
            if gate_open {
                det.last_visibility = visibility;
            }
        }

        // (2) Suspect accumulation + commit.
        let suspects_before = soldier.npc.detection_suspects[kind_idx];
        let suspects_after = suspects_before.saturating_add(sum_of_sharpnesses as u16);
        soldier.npc.detection_suspects[kind_idx] = suspects_after;
        let commit_threshold = suspects_after >= ai_vision::DETECTION_SUSPECT_THRESHOLD as u16
            || (instant_detection && sum_of_sharpnesses > 0);

        // (3) HandlePredetection shadow events for PC-typed targets.
        // Body is the only kind that fires; the helper is gated on
        // `fire_shadow_for_pc_targets` so the Friend / MissedFriend
        // / Beggar pre-empt and the Object skip fall out naturally.
        // Per-detectable rising edge of
        //   shadow_is_seen = (sharpness > 0)
        //                && (suspects[type] >= SHADOW_DETECTION_THRESHOLD)
        // — done before the `commit_threshold` resets suspects to 0,
        // so the pre-commit accumulator value drives the shadow gate.
        //
        // Skip PCs already in custody (guarded) — once a soldier is
        // guarding a hero, no further shadow events fire for that
        // hero on any detectable kind.
        let mut shadow_dispatches: Vec<crate::ai::Position> = Vec::new();
        if fire_shadow_for_pc_targets {
            for det in soldier.npc.detectable_lists[kind_idx].iter_mut() {
                // Only PCs are seen as shadows.
                let Some(target_id) = det.element else {
                    continue;
                };
                let Some(target) = targets.get(&target_id) else {
                    continue;
                };
                if !target.is_pc {
                    continue;
                }
                let shadow_is_seen =
                    det.seen_now && suspects_after as u32 >= ai_vision::SHADOW_DETECTION_THRESHOLD;
                let shadow_was_seen = det.shadow_seen_last_frame;
                det.shadow_seen_last_frame = shadow_is_seen;
                if shadow_is_seen && !shadow_was_seen && !target.guarded {
                    shadow_dispatches.push(crate::ai::Position {
                        x: target.position.x,
                        y: target.position.y,
                        sector: None,
                        level: 0,
                    });
                }
            }
        }

        // worst_detected_type bookkeeping — only on visibility
        // frames where new sharpness was added.
        if sum_of_sharpnesses > 0 && (soldier.npc.worst_detected_type as u8) > (kind as u8) {
            soldier.npc.worst_detected_type = kind;
        }

        // (4) Rising-edge dispatch + drop-on-commit.  When the threshold
        // or instant-detection commits, drop every detectable that
        // crossed the rising edge this frame and queue its event.
        let mut rising_dispatches: Vec<EntityId> = Vec::new();
        if commit_threshold {
            soldier.npc.detection_suspects[kind_idx] = 0;
            soldier.npc.detectable_lists[kind_idx].retain_mut(|det| {
                let Some(target_id) = det.element else {
                    return false;
                };
                if det.seen_now && !det.seen_last_frame {
                    rising_dispatches.push(target_id);
                    return false; // drop on commit
                }
                true
            });
        }

        // (5) Suspect cooldown when nothing visible.
        if sum_of_sharpnesses == 0
            && soldier.npc.detection_suspects[kind_idx] > 0
            && ctx
                .universal_frame
                .is_multiple_of(ai_vision::UNSUSPECT_FREQUENCY)
        {
            soldier.npc.detection_suspects[kind_idx] =
                soldier.npc.detection_suspects[kind_idx].saturating_sub(1);
        }

        // (6) maximal_detection_suspect contribution
        // (`type < FRIEND` only).
        if contribute_to_maximal
            && soldier.npc.maximal_detection_suspect < soldier.npc.detection_suspects[kind_idx]
        {
            soldier.npc.maximal_detection_suspect = soldier.npc.detection_suspects[kind_idx];
        }

        // (7) Drain the queued stimuli onto pending_stimuli.
        if (!rising_dispatches.is_empty() || !shadow_dispatches.is_empty())
            && let Some(ai) = soldier.npc.ai_brain.base_mut()
        {
            for _shadow_pos in &shadow_dispatches {
                tracing::trace!(
                    npc = ?npc_id,
                    ?kind,
                    "EventSeesShadow (rising edge)"
                );
            }
            for target_id in &rising_dispatches {
                tracing::trace!(
                    npc = ?npc_id,
                    target = ?target_id,
                    ?kind,
                    ?event_type,
                    "non-Enemy detectable rising edge"
                );
            }
            ai.pending_stimuli.extend(queued_human_detection_stimuli(
                event_type,
                shadow_dispatches,
                rising_dispatches,
            ));
        }
    }

    /// Per-NPC OBJECT detection — sibling of
    /// `run_human_detectable_pass` that calls
    /// `ai_vision::compute_object_visibility` instead of
    /// `compute_visibility`.  Same surrounding per-type loop
    /// machinery; no shadow events because the `IsPC()` gate skips
    /// objects.
    #[allow(clippy::too_many_arguments)]
    fn run_object_detectable_pass(
        soldier: &mut crate::element::ActorSoldier,
        npc_id: EntityId,
        frequency: u32,
        instant_detection: bool,
        targets: &std::collections::HashMap<EntityId, ObjectTarget>,
        ctx: ViewContext<'_>,
    ) {
        let obj_idx = DetectableType::Object as usize;
        let gate_open = ctx.refresh_always || ctx.modified_frame.is_multiple_of(frequency);

        // CleanUpDetectables for OBJECT: drop entries whose target
        // is no longer active.  Run before the visibility loop so
        // dead entries don't waste a tick of accumulator decay.
        soldier.npc.detectable_lists[obj_idx].retain(|det| {
            let Some(target_id) = det.element else {
                return false;
            };
            targets.get(&target_id).map(|o| o.active).unwrap_or(false)
        });

        let mut sum_of_sharpnesses: u32 = 0;

        for det in soldier.npc.detectable_lists[obj_idx].iter_mut() {
            let Some(target_id) = det.element else {
                det.seen_now = false;
                det.last_visibility = 0.0;
                continue;
            };
            let Some(object) = targets.get(&target_id) else {
                det.seen_now = false;
                det.last_visibility = 0.0;
                continue;
            };
            if object.layer != ctx.layer {
                det.seen_now = false;
                det.last_visibility = 0.0;
                continue;
            }
            let visibility: f32 = if ctx.viewer_in_building {
                0.0
            } else if gate_open {
                let q = ai_vision::ObjectVisibilityQuery {
                    viewer: ctx.eye,
                    viewer_direction: ctx.dir,
                    view_forward: ctx.view_forward,
                    view_radius: ctx.view_radius,
                    viewer_eye_status: ctx.eye_status,
                    real_half_aperture: ctx.real_half_aperture,
                    viewer_in_building: ctx.viewer_in_building,
                    object_belongs_to_beggar: object.belongs_to_beggar,
                    target: object.position,
                    sight_obstacles: *ctx.sight_obstacles,
                    fast_grid: ctx.fast_grid,
                    layer: ctx.layer,
                };
                frequency as f32 * ai_vision::compute_object_visibility(&q)
            } else {
                det.last_visibility
            };

            let sharpness = (ctx.view_speed as f32 * visibility) as u32;
            let is_visible = sharpness > 0;
            if !det.seen_last_frame {
                sum_of_sharpnesses = sum_of_sharpnesses.saturating_add(sharpness);
            }
            det.seen_now = is_visible;
            if gate_open {
                det.last_visibility = visibility;
            }
        }

        let suspects_after =
            soldier.npc.detection_suspects[obj_idx].saturating_add(sum_of_sharpnesses as u16);
        soldier.npc.detection_suspects[obj_idx] = suspects_after;
        let commit_threshold = suspects_after >= ai_vision::DETECTION_SUSPECT_THRESHOLD as u16
            || (instant_detection && sum_of_sharpnesses > 0);

        if sum_of_sharpnesses > 0
            && (soldier.npc.worst_detected_type as u8) > (DetectableType::Object as u8)
        {
            soldier.npc.worst_detected_type = DetectableType::Object;
        }

        let mut rising_dispatches: Vec<EntityId> = Vec::new();
        if commit_threshold {
            soldier.npc.detection_suspects[obj_idx] = 0;
            soldier.npc.detectable_lists[obj_idx].retain_mut(|det| {
                let Some(target_id) = det.element else {
                    return false;
                };
                if det.seen_now && !det.seen_last_frame {
                    rising_dispatches.push(target_id);
                    return false;
                }
                true
            });
        }

        if sum_of_sharpnesses == 0
            && soldier.npc.detection_suspects[obj_idx] > 0
            && ctx
                .universal_frame
                .is_multiple_of(ai_vision::UNSUSPECT_FREQUENCY)
        {
            soldier.npc.detection_suspects[obj_idx] =
                soldier.npc.detection_suspects[obj_idx].saturating_sub(1);
        }

        if soldier.npc.maximal_detection_suspect < soldier.npc.detection_suspects[obj_idx] {
            soldier.npc.maximal_detection_suspect = soldier.npc.detection_suspects[obj_idx];
        }

        if !rising_dispatches.is_empty()
            && let Some(ai) = soldier.npc.ai_brain.base_mut()
        {
            for target_id in rising_dispatches {
                let stimulus = crate::ai::Stimulus::with_human(
                    crate::ai::StimulusType::EventSeesObject,
                    target_id.index(),
                );
                ai.pending_stimuli.push(stimulus);
                tracing::trace!(
                    npc = ?npc_id,
                    object = ?target_id,
                    "EventSeesObject (rising edge)"
                );
            }
        }
    }
}

/// Read-only NPC view-state bundled for one tick of the per-type
/// detection passes (Body / Friend / MissedFriend / Beggar / Object).
/// Avoids passing 18+ args to each helper.  All fields are derived
/// from the soldier's npc/element state at the start of the per-NPC
/// pass; nothing here mutates.
struct ViewContext<'a> {
    eye: MapPoint,
    eye_z: f32,
    dir: i16,
    layer: u16,
    view_forward: (f32, f32),
    view_radius: u16,
    real_half_aperture: f32,
    viewer_in_building: bool,
    viewer_building_sector: Option<crate::position_interface::SectorHandle>,
    /// Hoisted once-per-viewer ground-radius — used as the fast path
    /// for any target that is not standing on a projection obstacle.
    effective_view_radius_ground: f32,
    /// Per-target obstacle-aware override.  Targets absent from
    /// this map fall back to `effective_view_radius_ground`.
    per_target_view_radius: &'a std::collections::HashMap<EntityId, f32>,
    eye_status: crate::element::EyeStatus,
    view_speed: u16,
    refresh_always: bool,
    modified_frame: u32,
    universal_frame: u32,
    golden_eye: bool,
    sight_obstacles: &'a crate::sight_obstacle::ObstacleList<'a>,
    fast_grid: &'a crate::fast_find_grid::FastFindGrid,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{Position, Substate};
    use crate::element::Posture;

    #[test]
    fn enemy_near_sender_uses_original_trainer_substate_and_time_gates() {
        for substate in [
            Substate::AttackingReactiontimeTurning,
            Substate::AttackingReactiontime,
        ] {
            assert!(attacking_reactiontime_enemy_near_enabled(
                false, substate, 500, 0
            ));
            assert!(!attacking_reactiontime_enemy_near_enabled(
                true, substate, 500, 0
            ));
        }

        for substate in [
            Substate::AttackingApproachToObserve,
            Substate::AttackingObserve,
        ] {
            assert!(attacking_reactiontime_enemy_near_enabled(
                false, substate, 199, 100
            ));
            assert!(!attacking_reactiontime_enemy_near_enabled(
                false, substate, 200, 100
            ));
        }

        assert!(!attacking_reactiontime_enemy_near_enabled(
            false,
            Substate::AttackingRunningToEnemy,
            100,
            100
        ));
    }

    #[test]
    fn enemy_near_sender_uses_original_box_and_postures() {
        let origin = MapPoint::new(100.0, 200.0);
        for posture in [
            Posture::Upright,
            Posture::Crouched,
            Posture::CarryingCorpse,
            Posture::HelpingToClimb,
            Posture::CarryingOnShoulders,
        ] {
            assert!(enemy_is_in_react_immediately_zone(
                origin,
                Position {
                    x: 150.0,
                    y: 170.0,
                    ..Position::default()
                },
                posture
            ));
        }

        assert!(!enemy_is_in_react_immediately_zone(
            origin,
            Position {
                x: 150.1,
                y: 200.0,
                ..Position::default()
            },
            Posture::Upright
        ));
        assert!(!enemy_is_in_react_immediately_zone(
            origin,
            Position {
                x: 100.0,
                y: 230.1,
                ..Position::default()
            },
            Posture::Upright
        ));
        assert!(!enemy_is_in_react_immediately_zone(
            origin,
            Position {
                x: 100.0,
                y: 200.0,
                ..Position::default()
            },
            Posture::Spy
        ));
    }

    #[test]
    fn enemy_near_sender_only_scans_list_them_and_preserves_order() {
        let origin = MapPoint::new(100.0, 200.0);
        let nearby = |x| Position {
            x,
            y: 200.0,
            ..Position::default()
        };
        let list_them = [3, 5, 1, 4];

        let selected = enemies_near_from_them_list(origin, &list_them, |handle| match handle {
            1 => Some((nearby(110.0), Posture::Upright)),
            // Handle 2 is nearby but deliberately absent from list_them.
            2 => Some((nearby(105.0), Posture::Upright)),
            3 => Some((nearby(151.0), Posture::Upright)),
            4 => Some((nearby(105.0), Posture::Spy)),
            5 => Some((nearby(95.0), Posture::Crouched)),
            _ => None,
        });

        assert_eq!(selected, vec![5, 1]);
    }

    #[test]
    fn body_predetection_shadow_is_queued_before_body_commit() {
        let stimuli = queued_human_detection_stimuli(
            crate::ai::StimulusType::EventSeesBody,
            vec![Position::default()],
            vec![EntityId::Soldier(crate::entity_id::SoldierId(7))],
        );
        assert_eq!(stimuli.len(), 2);
        assert_eq!(
            stimuli[0].stimulus_type,
            crate::ai::StimulusType::EventSeesShadow
        );
        assert_eq!(
            stimuli[1].stimulus_type,
            crate::ai::StimulusType::EventSeesBody
        );
    }
}
