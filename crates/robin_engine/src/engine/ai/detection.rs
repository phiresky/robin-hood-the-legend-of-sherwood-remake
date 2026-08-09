//! Owner-local visibility work: NPC blip detection and enemy → PC / royalist
//! → enemy `RefreshDetection` dispatch at the matching NPC creation boundary.
//! PC Listen performs its captured-length mixed reveal/Heard scan in the
//! selected PC owner slot; object discovery remains with its object owner.

use super::snapshots::{AiWorldView, HumanTarget, ObjectTarget};

#[cfg(test)]
thread_local! {
    static HEARD_CALLBACK_OBSERVER: std::cell::RefCell<Option<Box<dyn FnMut(&mut EngineInner, EntityId)>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_heard_callback_observer(
    observer: Option<Box<dyn FnMut(&mut EngineInner, EntityId)>>,
) {
    HEARD_CALLBACK_OBSERVER.with(|slot| *slot.borrow_mut() = observer);
}
use super::*;
use crate::ai::AiPerTickData;
use crate::ai_vision;
use crate::coordinates::{GroundPoint, MapPoint};
use crate::element::{Camp, Detectable, DetectableType, Entity, EntityId, Posture};

const DETECTION_FREQUENCY_BLIP: u32 = 16;
const BLIP_SUPER_DETECTION: f32 = 1.5;
const BLIP_ON_SHOULDERS_FACTOR: f32 = 1.3;
const BLIP_CONE_APERTURE_FACTOR: f32 = 1.0;

/// One live PC/soldier entry in an NPC's mixed Enemy detectable list.
/// Rebuilt at that NPC's creation slot so earlier NPC Think mutations are
/// visible, while preserving the list's own insertion order during the scan.
#[derive(Clone)]
struct EnemyOpticalTarget {
    id: EntityId,
    position: MapPoint,
    /// Owner-boundary feet position used by `Position(pEnemy)` in Think.
    /// This can be newer than `position` when Rust has deferred a later
    /// element slot's batched movement past the current NPC owner.
    planning_position: MapPoint,
    ground_position: GroundPoint,
    sector: Option<crate::position_interface::SectorHandle>,
    layer: u16,
    posture: crate::element::Posture,
    action_state: crate::element::ActionState,
    building_sector: Option<crate::position_interface::SectorHandle>,
    /// Absent for a dead target awaiting live-list cleanup, or for an
    /// incomplete non-detectable fixture posture. A live list entry must have
    /// this and fails contextually at the visibility read when it does not.
    eye_z: Option<f32>,
    /// `ComputeDetectionPoint` in world space. Absent alongside `eye_z`.
    detection_point: Option<crate::coordinates::WorldPoint3D>,
    ground_z: f32,
    /// 16-sector facing.  Only used for `LeaningOut`: the detection
    /// point projects `direction × 40` forward.
    direction: i16,
    active: bool,
    unconscious: bool,
    /// Whether the target is currently passing through a door — used
    /// by the same-building visibility short-circuit.
    passing_door: bool,
    /// The projection obstacle this NPC target is currently standing
    /// on.  Used by the per-target `compute_view_radius` re-call.
    obstacle_idx: Option<crate::position_interface::ObstacleHandle>,
    is_pc: bool,
    is_soldier: bool,
    dead: bool,
    hollow_man: bool,
    guarded: bool,
    detection_speed_in_forest: u16,
    detection_speed_in_city: u16,
    order_type: crate::order::OrderType,
    blipped: bool,
}

/// Eye point of a human, in both spaces the visibility code needs: the
/// projected map point used by the cone / spatial LOS tests, and the
/// world-space `ComputeEyesPoint` result the 3D opaque-reachability query
/// takes verbatim. The world point is returned rather than rebuilt from the
/// projection because projecting and un-projecting is not an exact round trip
/// in binary32, and the query endpoints are compared bit for bit.
pub(super) fn human_eye_point_for_visibility(
    entity: &Entity,
) -> (MapPoint, crate::coordinates::WorldPoint3D) {
    let Some(eye) = entity.compute_eyes_point(None) else {
        let position = entity.element_data().position();
        let position_map = entity.element_data().position_map();
        return (position_map, position);
    };
    let ground_z = entity.element_data().position().z;
    // `compute_eyes_point` returns world-space 3D, where the feet point is
    // `(map_x, map_y + ground_z, ground_z)`. Project with the *feet*
    // elevation so posture-dependent horizontal offsets survive while eye
    // height remains exclusively in the returned Z component. Projecting
    // with `eye.z` would fold eye height into Y and then count it again in
    // `VisibilityQuery`'s 3D distance.
    //
    // TODO(coord-parity): `VisibilityQuery` still uses one `MapPoint` for
    // both projected-map LOS and the original C++ world-XY distance vector.
    // Those spaces diverge when viewer and target ground elevations differ;
    // split the query into LOS points and world-horizontal points before
    // changing this established projection invariant.
    (visibility_eye_xy(eye, ground_z), eye)
}

fn visibility_eye_xy(eye: crate::coordinates::WorldPoint3D, ground_z: f32) -> MapPoint {
    MapPoint::from_world_xyz(eye.x, eye.y, ground_z)
}

#[inline]
fn detection_sharpness(view_speed: u16, visibility: f32) -> u16 {
    (view_speed as f32 * visibility) as u16
}

#[inline]
fn accumulate_detection_sharpness(sum: u16, sharpness: u16) -> u16 {
    sum.wrapping_add(sharpness)
}

/// Exact range half of `RHElementActorPC::SeesBlip`.
///
/// The Original subtracts the world-space `ComputeEyesPoint` results and only
/// then applies the isometric Y stretch. The same world-space eye points are
/// also passed to the Original's 3D opaque-reachability query.
fn sees_blip_in_range(
    pc_eye: crate::coordinates::WorldPoint3D,
    blip_eye: crate::coordinates::WorldPoint3D,
    standard_radius: f32,
    super_detection: f32,
) -> bool {
    let dx = blip_eye.x - pc_eye.x;
    let dy = (blip_eye.y - pc_eye.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
    let dz = blip_eye.z - pc_eye.z;

    if dz >= 0.0 {
        dx * dx + dy * dy + dz * dz
            < super_detection * super_detection * standard_radius * standard_radius
    } else {
        let horizontal_radius =
            super_detection * (standard_radius + BLIP_CONE_APERTURE_FACTOR * -dz);
        dx * dx + dy * dy < horizontal_radius * horizontal_radius
    }
}

/// Exact distance half of `RHElementActorPC::ListenTo`.
///
/// Original subtracts the elements' full `GetPosition()` world points and
/// only then applies the isometric Y stretch. Projected map Y omits the
/// elevation contribution to world Y and can therefore move elevated targets
/// across the strict 750-unit Listen boundary in either direction.
fn listen_distance_squared(
    listener: crate::coordinates::WorldPoint3D,
    target: crate::coordinates::WorldPoint3D,
) -> f32 {
    let dx = target.x - listener.x;
    let dy = (target.y - listener.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
    let dz = target.z - listener.z;
    dx * dx + dy * dy + dz * dz
}

struct SoldierSightContext {
    /// Literal owner position, before the AI `Position(actor)` door-side
    /// forecast used by shared entity views.
    position: crate::ai::Position,
    eye: MapPoint,
    /// World-space `ComputeEyesPoint` result, used verbatim as the origin of
    /// opaque-reachability queries.
    eye_world: crate::coordinates::WorldPoint3D,
    dir: i16,
    layer: u16,
    view_radius: u16,
    eye_status: crate::element::EyeStatus,
    current_state: crate::ai::AiState,
    current_substate: crate::ai::Substate,
    view_forward: (f32, f32),
    real_half_aperture: f32,
    posture: crate::element::Posture,
    action_state: crate::element::ActionState,
    sector: Option<crate::position_interface::SectorHandle>,
    alert_status: crate::ai::AlertLevel,
    blipped: bool,
    ground_position: GroundPoint,
    camp: Camp,
    ignore_bodies: bool,
}

fn lacklandist_visibility_refresh_always(
    eye_status: crate::element::EyeStatus,
    view_alert_status: crate::ai::AlertLevel,
) -> bool {
    matches!(
        eye_status,
        crate::element::EyeStatus::Stare | crate::element::EyeStatus::Follow
    ) || view_alert_status != crate::ai::AlertLevel::Green
}

/// Original's forest-wide rear-view exception is camp-based. Mounted
/// Royalists take the same 180-degree detection path as every other Royalist.
fn forest_180_degree_view_enabled(is_forest_level: bool, viewer_camp: Camp) -> bool {
    is_forest_level && viewer_camp == Camp::Royalists
}

/// Original clears the remembered worst type only after every detectable
/// bucket has contributed its persistent suspect value to the frame maximum.
fn finalize_detection_summary(npc: &mut crate::element::NpcData) {
    if npc.maximal_detection_suspect == 0 {
        npc.worst_detected_type = DetectableType::None;
    }
}

impl SoldierSightContext {
    fn from_npc_viewer(
        npc_id: EntityId,
        entity: &Entity,
        viewer_building_sector: Option<crate::position_interface::SectorHandle>,
    ) -> Option<Self> {
        let (npc, camp) = match entity {
            Entity::Soldier(soldier) => (&soldier.npc, soldier.soldier.cached_camp),
            Entity::Civilian(civilian) => (&civilian.npc, civilian.civilian.cached_camp),
            _ => return None,
        };
        // RefreshDetection's optical gate is narrower than its entry gate:
        // an inactive NPC in a BUILDING sector still scans, while an
        // inactive NPC merely passing a door stops after acoustics.
        if (!entity.is_active() && viewer_building_sector.is_none())
            || entity.is_dead()
            || entity.human_data().is_some_and(|human| human.unconscious)
            || entity.element_data().posture == crate::element::Posture::Tied
        {
            return None;
        }

        let ai = match entity {
            Entity::Soldier(_) => {
                &npc.ai_brain
                    .enemy()
                    .unwrap_or_else(|| {
                        panic!(
                            "eligible soldier NPC {} has no EnemyAi brain during detection",
                            npc_id.index()
                        )
                    })
                    .base
            }
            Entity::Civilian(_) => {
                &npc.ai_brain
                    .friendly()
                    .unwrap_or_else(|| {
                        panic!(
                            "eligible civilian NPC {} has no FriendlyAi brain during detection",
                            npc_id.index()
                        )
                    })
                    .base
            }
            _ => unreachable!("non-NPC passed the NPC viewer kind gate"),
        };
        let current_substate = ai.current_substate;
        let ignore_bodies = matches!(
            current_substate,
            crate::ai::Substate::SeekingOfficerWaitForAlertingSoldier
                | crate::ai::Substate::SeekingOfficerGetAlertingReportFromSoldier
        );
        let ground_position = entity.ground_position();
        let (eye, eye_world) = human_eye_point_for_visibility(entity);

        Some(Self {
            position: crate::ai::Position {
                x: entity.element_data().position_map().x,
                y: entity.element_data().position_map().y,
                sector: entity.element_data().sector(),
                level: entity.element_data().layer(),
            },
            eye,
            eye_world,
            dir: entity.element_data().direction(),
            layer: entity.element_data().layer(),
            view_radius: npc.view_radius,
            eye_status: npc.eye_status,
            current_state: npc.ai_state(),
            current_substate,
            view_forward: (npc.view_direction[0], npc.view_direction[1]),
            real_half_aperture: npc.real_half_aperture,
            posture: entity.element_data().posture,
            action_state: entity
                .actor_data()
                .map(|actor| actor.action_state)
                .unwrap_or(crate::element::ActionState::Waiting),
            sector: entity.element_data().sector(),
            // ComputeVisibility's refresh-always gate reads the view
            // parameters, not the independently tracked music alert. Shadow
            // sightings deliberately raise only the latter.
            alert_status: ai.view_alert_status,
            blipped: entity.element_data().blipped,
            ground_position,
            camp,
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
    target: MapPoint,
    posture: crate::element::Posture,
) -> bool {
    posture.triggers_enemy_near()
        && (target.x - origin.x).abs() <= 50.0
        && (target.y - origin.y).abs() <= 30.0
}

fn enemies_near_from_them_list(
    origin: MapPoint,
    list_them: &[u32],
    mut target_snapshot: impl FnMut(u32) -> Option<(MapPoint, crate::element::Posture)>,
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

fn refresh_detection_scans_target(
    last_visibility: f32,
    viewer_inside_building: bool,
    viewer_position: GroundPoint,
    view_radius: u16,
    target_position: GroundPoint,
) -> bool {
    if last_visibility > 0.0 || viewer_inside_building {
        return true;
    }

    let radius_x = view_radius as f32;
    let radius_y = radius_x * crate::position_interface::ASPECT_RATIO;
    (target_position.x - viewer_position.x).abs() <= radius_x
        && (target_position.y - viewer_position.y).abs() <= radius_y
}

fn non_enemy_visibility_blocked_before_cadence(
    eye_status: crate::element::EyeStatus,
    viewer_camp: Camp,
    type_gate_blocked: bool,
) -> bool {
    eye_status.is_blind() || viewer_camp != Camp::Lacklandists || type_gate_blocked
}

fn missed_friend_or_beggar_target_blocked(dead: bool, unconscious: bool) -> bool {
    dead || unconscious
}

fn apply_enemy_beggar_disguise(
    viewer_camp: Camp,
    target_is_pc: bool,
    got_beggar_trick: &mut bool,
    order_type: crate::order::OrderType,
    visibility: f32,
) -> f32 {
    if viewer_camp != Camp::Lacklandists || !target_is_pc || *got_beggar_trick || visibility <= 0.0
    {
        return visibility;
    }

    match order_type {
        crate::order::OrderType::SimulatingBeggar => 0.0,
        crate::order::OrderType::TransitionWaitingUprightSimulatingBeggar
        | crate::order::OrderType::TransitionSimulatingBeggarWaitingUpright => {
            *got_beggar_trick = true;
            visibility
        }
        _ => visibility,
    }
}

/// Original `RHElementActorNPC::HandlePredetection` shadow-edge update.
///
/// The shadow threshold is tested against the suspect accumulator as it stood
/// before the current scan. The caller adds this frame's sharpness only after
/// every detectable has passed through this helper.
fn update_predetection_shadow_latch(
    seen_now: bool,
    suspects_before_scan: u16,
    is_pc: bool,
    guarded: bool,
    shadow_seen_last_frame: &mut bool,
) -> bool {
    // The Original returns before touching the latch for both cases.
    if !is_pc || guarded {
        return false;
    }

    let shadow_is_seen =
        seen_now && suspects_before_scan as u32 >= ai_vision::SHADOW_DETECTION_THRESHOLD;
    let shadow_was_seen = *shadow_seen_last_frame;
    *shadow_seen_last_frame = shadow_is_seen;
    shadow_is_seen && !shadow_was_seen
}

/// Staggered cadence counter used by `RHElementActorNPC::RefreshDetection`.
///
/// Original stores `universal frame + creation order` in a `UWORD` before
/// applying every blip, sound, and optical modulo gate. The truncation is
/// observable once a mission's universal frame passes 65535.
fn refresh_detection_modified_frame(universal_frame: u32, creation_order: u32) -> u32 {
    universal_frame.wrapping_add(creation_order) as u16 as u32
}

/// Original's per-type `HandleDetection` cooldown. The sum is fresh
/// sharpness only: a visible target that is already latched contributes zero.
fn cool_detection_suspect(sum_of_sharpnesses: u16, suspect: u16, universal_frame: u32) -> u16 {
    if sum_of_sharpnesses == 0
        && suspect > 0
        && universal_frame.is_multiple_of(ai_vision::UNSUSPECT_FREQUENCY)
    {
        suspect.saturating_sub(1)
    } else {
        suspect
    }
}

impl EngineInner {
    /// Return the exact `RHElement::mulCreationOrder` assigned by the
    /// Original-compatible construction stream.
    fn original_static_creation_order(&self, entity_id: EntityId) -> u32 {
        self.world.original_creation_order(entity_id)
    }

    /// Original: `RHArtificialMalignity::AttackingReactiontimeEnemyNearTest`.
    ///
    /// `RHElementActorSoldier::Hourglass` calls this before the NPC detection
    /// pass. The gate is evaluated once, then the current `mlistThem` is
    /// walked in order and each eligible nearby enemy is sent through Think.
    pub(crate) fn tick_attacking_reactiontime_enemy_near_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        scratch: &SimScratch,
        npc_id: EntityId,
    ) {
        let frame = self.control.frame_counter;
        let Some(Entity::Soldier(soldier)) = self.world.entities.get(npc_id) else {
            panic!("EnemyNear owner {} disappeared", npc_id.index());
        };
        if !soldier.element.active {
            return;
        }
        let Some(enemy_ai) = soldier.npc.ai_brain.enemy() else {
            return;
        };
        if !attacking_reactiontime_enemy_near_enabled(
            enemy_ai.combat_trainer,
            enemy_ai.base.current_substate,
            frame,
            enemy_ai.base.frame_when_enemy_detected,
        ) {
            return;
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
            // `AttackingReactiontimeEnemyNearTest` reads the element's
            // literal `GetPositionMap()`.  `view.position` is AI
            // `Position(target)`, which forecasts a passing actor onto the
            // destination side of its door and can put it inside the 50x30
            // reaction box several frames too early.
            target_view.map(|view| (view.detection_position, view.posture))
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
            let building_sector = self
                .world
                .entities
                .get(npc_id)
                .and_then(|entity| self.entity_building_sector(entity.element_data().sector()));
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
                self.control.sim_config.difficulty,
            );
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            let tick_data =
                self.build_npc_tick_data_for_target(sim, npc_id, scratch, assets, Some(target_id));
            let stimulus = crate::ai::Stimulus::with_human(
                crate::ai::StimulusType::EventEnemyNear,
                target_handle,
            );
            self.dispatch_think_with_drain(sim, npc_id, &stimulus, &ctx, &tick_data, assets);
        }
    }

    /// P2a — non-NPC blip work: drive the Listen ability's one-shot reveal,
    /// and FX-target Heard() callbacks. Ordinary NPC
    /// `SeesBlip` runs inside that NPC's creation-ordered RefreshDetection.
    pub(super) fn tick_enemy_ai_blip_detection(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        pc_id: EntityId,
    ) -> bool {
        const DISTANCE_LISTEN: f32 = 750.0;
        const TIME_LISTEN_WAIT: u32 = 25;
        // ── Listen ability frame tick. ──────────────────────
        // Each frame a PC is in `ListenPhase::CountingDown`:
        //
        //  - Arm `listen_wait_time` to `TIME_LISTEN_WAIT` on the
        //    first observation.
        //  - Decrement the countdown. On a nonterminal frame, call `Turn()`
        //    and drive the `LISTENING` sprite while deliberately ignoring its
        //    completion state. On the frame the countdown reaches 0,
        //    fire the one-shot blip reveal + FX-target `Heard()`
        //    callback (below) and advance the phase to
        //    `ExitTransition` so owner-local `tick_ability` plays the exit
        //    transition animation and cleans up the ability.
        //
        // The action state stays `Listening` through the
        // countdown — the exit transition in owner-local `tick_ability`
        // will flip it back to `Waiting`.
        #[derive(Clone, Copy)]
        struct FiringListener {
            position: crate::coordinates::WorldPoint3D,
            pc_id: EntityId,
            seq_id: crate::sequence::SequenceId,
            elem_idx: usize,
        }
        let firing_listener = {
            let pc = match self.world.entities.get_mut(pc_id) {
                Some(Entity::Pc(pc)) => pc,
                Some(_) => panic!("Listen owner {pc_id:?} is not a PC"),
                None => panic!("Listen owner {pc_id:?} disappeared"),
            };
            if pc.actor.listen_phase != crate::element::ListenPhase::CountingDown {
                return false;
            }
            if pc.actor.listen_wait_time == 0 {
                // First frame in the CountingDown phase — arm the
                // countdown. Original stores this in the actor's single
                // serialized `mulWaitTime`; the phase-local field is only a
                // Rust control-flow mirror and must remain synchronized.
                pc.actor.listen_wait_time = TIME_LISTEN_WAIT;
                pc.actor.wait_time = TIME_LISTEN_WAIT;
                pc.actor.seek_refresh_wait = TIME_LISTEN_WAIT;
            }
            pc.actor.listen_wait_time -= 1;
            if pc.actor.wait_time != 0 {
                pc.actor.wait_time -= 1;
            }
            pc.actor.seek_refresh_wait = pc.actor.wait_time;
            if pc.actor.listen_wait_time != 0 {
                // RHElementActorPC::Execute performs the visual action only
                // after the timer's terminal early return. Its sprite result
                // never advances the sequence; mulWaitTime is authoritative.
                pc.element.sprite.position_iface.turn();
                let direction = pc.element.direction() as u16;
                let order_id = pc
                    .actor
                    .active_ability
                    .order_id
                    .expect("Listening phase has a current order");
                let _ignored_motion = pc.element.sprite.perform_action(
                    sim,
                    Some(order_id),
                    crate::order::OrderType::Listening,
                    direction,
                    crate::sprite::FrameProgression::Default,
                    false,
                );
                // RHElementActorPC::Execute deliberately discards
                // PerformAction's START/DONE result for LISTENING and returns
                // RHMOTION_IN_PROGRESS on every nonterminal countdown tick.
                // The actor continuation stores that wrapper result, not the
                // raw sprite edge.
                // The owner coordinator latches the specialized Execute
                // result from `last_motion_state` after this helper returns.
                // Store the PC wrapper's authoritative result there; merely
                // changing `continuation` here would be overwritten by the
                // raw sprite START/DONE edge later in the same actor slot.
                pc.element.sprite.last_motion_state = Some(crate::sprite::MotionState::InProgress);
                return false;
            }
            // Countdown hit 0 — fire the one-shot reveal and
            // advance the phase so owner-local `tick_ability` plays the
            // exit transition next.
            let fl = FiringListener {
                pc_id,
                position: pc.element.position(),
                seq_id: pc
                    .actor
                    .active_ability
                    .sequence_id
                    .expect("Listen sequence"),
                elem_idx: pc.actor.active_ability.element_index,
            };
            tracing::debug!(
                pc = pc_id.index(),
                "Listen: one-shot reveal fired after TIME_LISTEN_WAIT frames"
            );
            fl
        };

        let listener = firing_listener;
        {
            // C++ captures Size() once, then resolves each live slot and
            // applies RevealBlip/Heard synchronously in that mixed order.
            let captured_len = self.world.entities.len();
            for slot in 0..captured_len {
                let Some(entity_id) = self.world.entities.id_at_legacy_slot(slot as u32) else {
                    continue;
                };
                let Some(entity) = self.world.entities.get(entity_id) else {
                    continue;
                };
                let elem = entity.element_data();
                if listen_distance_squared(listener.position, elem.position())
                    >= DISTANCE_LISTEN * DISTANCE_LISTEN
                {
                    continue;
                }
                let reveal = elem.blipped
                    && matches!(
                        entity,
                        Entity::Soldier(_)
                            | Entity::Civilian(_)
                            | Entity::Bonus(_)
                            | Entity::Scroll(_)
                            | Entity::Projectile(_)
                            | Entity::Net(_)
                    );
                let heard = matches!(entity, Entity::Target(_));
                if reveal {
                    self.world
                        .entities
                        .get_mut(entity_id)
                        .unwrap()
                        .reveal_blip();
                }
                if heard && sim.config().script_enabled {
                    let target = match self.world.entities.get_mut(entity_id) {
                        Some(Entity::Target(target)) => target,
                        _ => panic!("Listen target {entity_id:?} changed type before Heard"),
                    };
                    if !target
                        .target
                        .action_filter
                        .contains(crate::element::TargetFilter::LISTEN)
                    {
                        continue;
                    }
                    target
                        .target
                        .action_filter
                        .remove(crate::element::TargetFilter::LISTEN);
                    assert!(
                        !target.target.script_class.is_empty(),
                        "LISTEN target {entity_id:?} has no required script class"
                    );
                    let target_handle = crate::natives::ScriptHandleCodec::actor_handle(entity_id);
                    let pc_handle = crate::natives::ScriptHandleCodec::actor_handle(listener.pc_id);
                    self.call_script_vm(
                        sim,
                        assets,
                        ScriptVmKey::Target(target_handle),
                        "ActivatedByListenable",
                        &[pc_handle],
                        crate::natives::ScriptCallFrame::actor(target_handle),
                    )
                    .unwrap_or_else(|error| {
                        panic!("ActivatedByListenable target {target_handle} failed: {error}")
                    });
                    #[cfg(test)]
                    HEARD_CALLBACK_OBSERVER.with(|observer| {
                        if let Some(observer) = observer.borrow_mut().as_mut() {
                            observer(self, entity_id);
                        }
                    });
                }
            }
            self.do_next_order(listener.seq_id, listener.elem_idx);
            let (exit_order_id, exit_order_type) = self
                .orders
                .sequence_manager
                .get_element(listener.seq_id, listener.elem_idx)
                .and_then(|element| element.current_order())
                .map(|order| (order.order_id, order.order_type))
                .unwrap_or_else(|| panic!("Listen countdown did not expose its exit order"));
            assert_eq!(
                exit_order_type,
                crate::order::OrderType::TransitionListeningWaitingUpright
            );
            let actor = self
                .get_entity_mut(listener.pc_id)
                .and_then(Entity::actor_data_mut)
                .expect("Listen owner vanished after synchronous scan");
            actor.listen_phase = crate::element::ListenPhase::ExitTransition;
            actor.active_ability.order_id = Some(exit_order_id);
            actor.active_ability.done_effect_applied = false;
        }
        true
    }

    /// Strict live `RHElementBonus::RefreshDiscovered` for one bonus-owned
    /// virtual Hourglass slot.
    pub(crate) fn refresh_bonus_discovered_for(
        &mut self,
        assets: &LevelAssets,
        bonus_id: EntityId,
    ) {
        let bonus =
            self.world.entities.get(bonus_id).unwrap_or_else(|| {
                panic!("bonus {bonus_id:?} disappeared before RefreshDiscovered")
            });
        let Entity::Bonus(bonus) = bonus else {
            panic!("RefreshDiscovered owner {bonus_id:?} is not Entity::Bonus")
        };
        if !bonus.element.blipped || !bonus.element.active {
            return;
        }
        let bonus_position = bonus.element.position();
        let radius = self.ai.standard_view_polygon_radius as f32;
        let square_standard_view_radius = radius * radius;
        let pc_ids = self.world.original_pc_registry_ids.clone();
        let sight_obstacles = self.sight_obstacles(assets);
        let discovered = pc_ids.into_iter().any(|pc_id| {
            let entity = self.world.entities.get(pc_id).unwrap_or_else(|| {
                panic!("bonus {bonus_id:?} RefreshDiscovered found stale PC registry id {pc_id:?}")
            });
            let Entity::Pc(pc) = entity else {
                panic!("bonus {bonus_id:?} RefreshDiscovered found non-PC registry id {pc_id:?}")
            };
            if pc.pc.life_points <= 0 || pc.human.unconscious || !pc.element.active {
                return false;
            }
            let eyes = entity.compute_eyes_point(None).unwrap_or_else(|| {
                panic!("bonus {bonus_id:?} could not compute eyes for required PC {pc_id:?}")
            });
            let dx = eyes.x - bonus_position.x;
            let dy = (eyes.y - bonus_position.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = eyes.z - bonus_position.z;
            let threshold = if pc.element.posture == crate::element::Posture::OnShoulders {
                1.3
            } else {
                1.0
            } * square_standard_view_radius;
            dx * dx + dy * dy + dz * dz < threshold
                && crate::sight_obstacle::is_reachable_3d(
                    sight_obstacles,
                    [bonus_position.x, bonus_position.y, bonus_position.z],
                    [eyes.x, eyes.y, eyes.z],
                    crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
                )
        });
        if discovered {
            self.world
                .entities
                .get_mut(bonus_id)
                .expect("discovered bonus disappeared before clearing its blip")
                .reveal_blip();
        }
    }

    /// NPC-owned `RefreshDetection` blip arm. This must run at the start of
    /// each NPC's creation slot: an earlier NPC's synchronous Think/script
    /// may activate, deactivate, blip, reveal, or move a later NPC before its
    /// own cadence opens.
    fn tick_enemy_ai_npc_blip_detection_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        assets: &LevelAssets,
    ) {
        use crate::element::Posture;

        let entity = self.world.entities.get(npc_id).unwrap_or_else(|| {
            panic!(
                "creation-ordered NPC {} disappeared before its blip detection slot",
                npc_id.index()
            )
        });
        let elem = entity.element_data();
        if !elem.blipped
            || !(elem.active
                || elem.is_in_door_transit()
                || self.entity_building_sector(elem.sector()).is_some())
            || !refresh_detection_modified_frame(
                self.control.frame_counter,
                self.original_static_creation_order(npc_id),
            )
            .is_multiple_of(DETECTION_FREQUENCY_BLIP)
        {
            return;
        }

        // Royalist soldiers reveal themselves without consulting PCs, but
        // only behind the same RefreshDetection entry/cadence gates.
        if matches!(entity, Entity::Soldier(s) if s.soldier.cached_camp == Camp::Royalists) {
            self.world
                .entities
                .get_mut(npc_id)
                .expect("blipped Royalist NPC disappeared before reveal")
                .reveal_blip();
            return;
        }

        let (_blip_eye_xy, blip_eye_world) = human_eye_point_for_visibility(entity);
        let standard_radius = if self.ai.standard_view_polygon_radius > 0 {
            self.ai.standard_view_polygon_radius as f32
        } else {
            ai_vision::DEFAULT_VIEW_RADIUS as f32
        };
        let difficulty_factor = match sim.config().difficulty {
            crate::player_profile::DifficultyLevel::Easy => {
                crate::player_profile::difficulty_params::EASY_BLIP_DETECTION_RANGE
            }
            crate::player_profile::DifficultyLevel::Medium => 1.0,
            crate::player_profile::DifficultyLevel::Hard => {
                crate::player_profile::difficulty_params::HARD_BLIP_DETECTION_RANGE
            }
        };
        let sight_obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };

        let mut detecting_pc = None;
        let pc_ids = self.world.original_pc_registry_ids.clone();
        for pc_id in pc_ids {
            let pc_entity = self.world.entities.get(pc_id).unwrap_or_else(|| {
                panic!(
                    "PC {} disappeared from the live PC list during NPC blip detection",
                    pc_id.index()
                )
            });
            let Entity::Pc(pc) = pc_entity else {
                panic!(
                    "non-PC entity {} is present in the live PC list during NPC blip detection",
                    pc_id.index()
                );
            };
            if !pc.element.active
                || !pc.pc.playable
                || pc.pc.life_points <= 0
                || pc.human.unconscious
            {
                continue;
            }
            let (_pc_eye_xy, pc_eye_world) = human_eye_point_for_visibility(pc_entity);
            let super_detection = if pc.element.posture == Posture::OnShoulders {
                BLIP_SUPER_DETECTION * BLIP_ON_SHOULDERS_FACTOR
            } else {
                BLIP_SUPER_DETECTION
            } * difficulty_factor;
            let in_range = sees_blip_in_range(
                pc_eye_world,
                blip_eye_world,
                standard_radius,
                super_detection,
            );
            if in_range
                && crate::sight_obstacle::is_reachable_3d(
                    sight_obstacles,
                    [pc_eye_world.x, pc_eye_world.y, pc_eye_world.z],
                    [blip_eye_world.x, blip_eye_world.y, blip_eye_world.z],
                    crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
                )
            {
                detecting_pc = Some((pc_id, pc.element.posture == Posture::OnShoulders));
                break;
            }
        }

        let Some((pc_id, perched)) = detecting_pc else {
            return;
        };
        self.world
            .entities
            .get_mut(npc_id)
            .expect("blipped NPC disappeared before reveal")
            .reveal_blip();
        if perched {
            self.hero_speaking(
                assets,
                pc_id,
                crate::engine::melee::HERO_PERCHED_AND_SEE_ENNEMY,
            );
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
        sim: &crate::sim_rng::SimulationContext,
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
        let (position_map, position_world, current_state) = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            // RefreshDetection's first gate admits inactive NPCs only while
            // they have a door pointer or an actual BUILDING sector. This is
            // deliberately broader than the later sector-only optical gate.
            if !entity.is_active()
                && !entity.element_data().is_in_door_transit()
                && self
                    .entity_building_sector(entity.element_data().sector())
                    .is_none()
            {
                return;
            }
            // Every NPC runs the acoustic pass — it lives on the base NPC
            // class. Which PCs it considers is determined exclusively by its
            // authoritative DETECTABLE_ENEMY list below.
            if !matches!(entity, Entity::Civilian(_) | Entity::Soldier(_)) {
                return;
            }
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
                entity.element_data().position(),
                npc.ai_state(),
            )
        };
        // Attacking NPCs are already locked onto their target
        // and don't accumulate new hearing stimuli.
        if matches!(current_state, AiState::Attacking) {
            return;
        }
        let modified_frame = refresh_detection_modified_frame(
            universal_frame,
            self.original_static_creation_order(npc_id),
        );
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
            .max_noise_covering_volume_for_3d(position_world.x, position_world.y, position_world.z);

        let pc_target_ids = {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            let Some(npc) = entity.npc_data_mut() else {
                return;
            };
            let enemy_idx = DetectableType::Enemy as usize;

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
            pc_target_ids
        };

        let enemy_idx = DetectableType::Enemy as usize;
        for pc_id in pc_target_ids {
            let Some(pc) = world.pcs.iter().find(|pc| pc.id == pc_id) else {
                // A dead PC can remain in the detectable list until optical
                // CleanUpDetectables later in this same RefreshDetection
                // call. There is no acoustic snapshot to sample in that
                // expected stale window. Every living PC, including an
                // inactive one, must be present in the world view.
                match self.world.entities.get(pc_id) {
                    Some(entity) if entity.is_dead() => continue,
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
                    // Hear-my-noise-box pre-filter. Original stores this box
                    // on the PC and does not rebuild it when
                    // RefreshProducedNoise returns through its
                    // inactive/building or quiet-animation arms. It can thus
                    // intentionally disagree with the current noise origin
                    // and volume; outside the stale box UpdateHearing is not
                    // called and the edge latch remains untouched.
                    let noise = pc.produced_noise;
                    if !pc.hear_noise_box.contains_point(position_map) {
                        None
                    } else {
                        // GetHearVolume uses the full 3D position. Its noise
                        // origin is `(x, y + elevation, elevation)` and it has
                        // no logical-layer rejection, so nearby cross-layer
                        // sounds remain audible when their actual geometry is.
                        let source_elevation = noise.elevation as f32;
                        let dy_stretched = (position_world.y - noise.origin.y - source_elevation)
                            * crate::position_interface::INVERSE_ASPECT_RATIO;
                        let dx_3d = position_world.x - noise.origin.x;
                        let dz = position_world.z - source_elevation;
                        let modified_volume = pc_volume as f32 * HEARING_FACTOR;
                        let max_norm = dx_3d.abs().max(dy_stretched.abs()).max(dz.abs());
                        let distance =
                            (dx_3d * dx_3d + dy_stretched * dy_stretched + dz * dz).sqrt();
                        // Original GetHearVolume explicitly rejects NOISE_OFF,
                        // a coincident source/listener, and sources beyond the
                        // modified-volume max norm. UpdateHearing still runs
                        // for all of these inside-box cases and clears its
                        // rising-edge latch.
                        let subjective = if pc_volume == 0
                            || distance == 0.0
                            || max_norm > modified_volume
                            || modified_volume - distance <= 0.0
                        {
                            0
                        } else {
                            // GetHearVolume reaches GetDeafness only after
                            // every semantic/range check and the positive
                            // subjective-volume test. Besides avoiding wasted
                            // work, this preserves the observable cached-frame
                            // mutation when all tracked PCs are inaudible.
                            let deafness = npc.get_deafness(universal_frame, cover_volume);
                            subjective_hear_volume(modified_volume, distance, deafness)
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

                        let stimulus = if subjective > 0 && !det_heard && !det_seen {
                            let noise = crate::ai::Noise {
                                origin: noise.origin,
                                noise_type: if pc.is_swordfighting {
                                    crate::ai::NoiseType::ZingZing
                                } else {
                                    crate::ai::NoiseType::TapTapTap
                                },
                                volume: subjective,
                                elevation: noise.elevation,
                                element_id: noise.element_id,
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
                        det.heard_last_frame = subjective > 0;
                        stimulus
                    }
                }
            };

            let Some(mut stimulus) = stimulus else {
                continue;
            };

            // `UpdateHearing` calls Think inline. Rebuild both views for each
            // edge because an earlier PC's hearing handler may mutate state
            // consumed by the next handler or by optical detection below.
            let scratch = self.build_sim_scratch(sim, assets);
            let source_position = scratch
                .ai_entity_views
                .get(&pc_id.index())
                .unwrap_or_else(|| {
                    panic!(
                        "heard PC {} vanished before UpdateHearing payload construction",
                        pc_id.index()
                    )
                })
                .position;
            let crate::ai::StimulusInfo::Noise(ref mut heard_noise) = stimulus.info else {
                panic!("periodic hearing edge lost its required noise payload")
            };
            // UpdateHearing constructs a fresh event and assigns
            // Position(pEnemy), which is the AI planning position (including
            // committed door-side and carrier substitution), not the raw
            // produced-noise origin used by GetHearVolume above.
            heard_noise.origin = source_position;
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
                self.control.sim_config.difficulty,
            );
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            let tick_data = self.build_npc_tick_data(sim, npc_id, &scratch, assets);
            self.dispatch_think_with_drain(sim, npc_id, &stimulus, &ctx, &tick_data, assets);
        }
    }

    /// P3 — per-NPC `RefreshDetection` pass.
    ///
    /// For every NPC: run synchronous acoustics, select the camp-specific
    /// Enemy visibility path (Lacklandist→PC or Royalist→Lacklandist), then run
    /// the remaining detectable buckets and flush that NPC's complete FIFO
    /// before advancing to the next creation slot. EVENT_VIEW is queued after
    /// the Enemy scan and dispatched only after every detectable bucket has
    /// released the NPC borrow.
    /// Volatile NPC target metadata is rebuilt at each creation slot so a
    /// later NPC observes state changes made by an earlier NPC's Think.
    /// Original:
    /// `RHelementactornpc.cpp::RefreshDetection` queues detection stimuli while
    /// scanning lists, then calls `Think` before returning from that NPC's
    /// Hourglass.
    pub(super) fn tick_enemy_ai_refresh_detection(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        world: &AiWorldView,
        positions_before_movement: Option<&EntitySlots<Option<crate::entities::BoundaryPosition>>>,
        owner: Option<EntityId>,
        dispatch_legacy_test_wakes: bool,
    ) {
        let universal_frame = self.control.frame_counter;
        let golden_eye = self.ai.global.golden_eye_mode;
        // Forest-level flag — selects between forest and city
        // detection-speed parameters when scaling a PC's visual
        // detection speed in the per-target visibility pass below.
        let is_forest_level = self.world.weather.is_forest_level;
        let npc_ids: Vec<_> = match owner {
            Some(npc_id) => vec![npc_id],
            None => self.world.entities.npc_ids().collect(),
        };

        for npc_id in npc_ids {
            // Original `RHElementActorNPC::Hourglass` performs these owner
            // operations immediately before this same NPC enters
            // `RefreshDetection` (`RHelementactornpc.cpp:3534-3546`). Do not
            // pre-apply a later NPC's body/recovery/view work: synchronous
            // broadcasts and Think/script effects from earlier slots may
            // affect later observers, never observers whose slots already ran.
            if let Some(positions_before_movement) = positions_before_movement {
                // The production owner envelope dispatches concussion wakes in
                // the Human pre-Actor hook. Keep the historical test-only
                // coordinator behavior for tests that call this lower-level
                // seam directly.
                if dispatch_legacy_test_wakes
                    && self.dispatch_pending_fit_again_for_npc(sim, npc_id, assets)
                {
                    self.tick_ai_pending_resurrection_and_eyes_for_npc(npc_id);
                    self.apply_wake_redetection_blinks(npc_id);
                }
                self.tick_inform_my_friends_for_npc(npc_id);
                self.refresh_npc_view_for_npc(npc_id, positions_before_movement);
            }

            self.tick_enemy_ai_npc_blip_detection_for_npc(sim, npc_id, assets);

            // Sample the two pre-acoustic RefreshDetection gates before
            // EVENT_HEAR can synchronously run Think/script and mutate the
            // viewer. Once these gates pass, original control flow always
            // reaches the pre-optical maxima reset.
            let passed_pre_acoustic_gates = self.world.entities.get(npc_id).is_some_and(|entity| {
                let elem = entity.element_data();
                let entered_refresh = elem.active
                    || elem.is_in_door_transit()
                    || self.entity_building_sector(elem.sector()).is_some();
                entered_refresh
                    && !entity.is_dead()
                    && entity.human_data().is_none_or(|human| !human.unconscious)
                    && elem.posture != Posture::Tied
            });
            self.tick_enemy_ai_acoustic_detection_for_npc(sim, npc_id, assets, world);

            // RefreshDetection clears both maxima after acoustics but
            // before its narrower optical eligibility gate. In particular,
            // an inactive NPC on a door rail reaches this reset and then
            // returns without scanning; an inactive outdoor NPC returned at
            // the entry gate and must retain the old value.
            if passed_pre_acoustic_gates
                && let Some(npc) = self
                    .world
                    .entities
                    .get_mut(npc_id)
                    .and_then(Entity::npc_data_mut)
            {
                npc.maximal_detection_suspect = 0;
                if let Some(ai) = npc.ai_brain.base_mut() {
                    ai.max_visibility = 0;
                }
            }

            // Original CleanUpDetectables/ComputeVisibility dereference live
            // human pointers. Rebuild the target records at this creation
            // slot, but let the NPC's detectable list dictate scan order.
            let enemy_targets = self.tick_enemy_ai_build_live_enemy_optical_targets(world);
            // Original caches ComputeViewRadius for this viewer/frame: one
            // ground entry plus one entry on each projection obstacle. Enemy
            // and the later detectable-type buckets share the same cache
            // during this contiguous RefreshDetection call.
            let view_radius_cache = OwnerViewRadiusCache::from_persistent(
                &self.ai.view_radius_cache,
                npc_id,
                universal_frame,
            );
            let think_input = self.tick_enemy_ai_refresh_detection_for_npc(
                npc_id,
                assets,
                world,
                &enemy_targets,
                universal_frame,
                golden_eye,
                is_forest_level,
                &view_radius_cache,
            );
            // Enemy HandlePredetection may already have queued shadows. Append
            // the ordered Enemy VIEW / OUTOFVIEW block now, before later
            // detectable types, preserving the original
            // SHADOW → (VIEW|OUTOFVIEW)* → BODY → OBJECT → FRIEND →
            // MISSED_FRIEND → BEGGAR FIFO.
            let enemy_block = think_input;
            let enemy_detection_tick_data = if let Some((stimuli, mut tick_data)) = enemy_block {
                self.prepare_detection_forecasts_for_owner(
                    npc_id,
                    positions_before_movement,
                    &mut tick_data,
                );
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
                let queue_start = ai.outbox.detection.stimuli.len();
                ai.outbox.detection.stimuli.extend(stimuli.iter().copied());
                Some(super::post_detection::PendingEnemyDetectionTickData::new(
                    queue_start,
                    stimuli,
                    tick_data,
                ))
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
            let (human_targets, object_targets) = self
                .tick_enemy_ai_build_human_object_targets_for_npc(
                    npc_id,
                    positions_before_movement,
                );
            self.tick_enemy_ai_refresh_per_type_for_npc(
                npc_id,
                assets,
                &human_targets,
                &object_targets,
                universal_frame,
                golden_eye,
                &view_radius_cache,
            );
            // No other viewer can run inside this contiguous
            // RefreshDetection scan. Commit at its boundary before the first
            // queued Think, where synchronous IsDetecting may consume it.
            view_radius_cache.commit_to(&mut self.ai.view_radius_cache, npc_id, universal_frame);

            let has_pending_stimuli = self
                .world
                .entities
                .get(npc_id)
                .and_then(Entity::ai_controller)
                .is_some_and(|ai| !ai.outbox.detection.stimuli.is_empty());
            if !has_pending_stimuli {
                assert!(
                    enemy_detection_tick_data.is_none(),
                    "queued Enemy detection block lost its stimuli before the per-NPC drain"
                );
            } else {
                self.tick_enemy_ai_drain_pending_stimuli_for_npc(
                    sim,
                    npc_id,
                    assets,
                    enemy_detection_tick_data,
                    positions_before_movement,
                );
            }

            // Production NPC Hourglass continues with this same owner's
            // complete tail before advancing to the next creation slot. The
            // focused detection-only seam passes no pre-movement positions
            // and deliberately stops at the RefreshDetection boundary.
            if positions_before_movement.is_some() {
                self.tick_npc_post_detection_tail_for_npc(sim, npc_id, assets);
            }
        }
    }

    /// Prepare destination forecast alternatives without drawing RNG. The AI
    /// handler that actually consumes a primary/missed/officer forecast owns
    /// any building-exit selection draw.
    pub(super) fn prepare_detection_forecasts_for_owner(
        &self,
        npc_id: EntityId,
        positions_before_movement: Option<&EntitySlots<Option<crate::entities::BoundaryPosition>>>,
        tick_data: &mut AiPerTickData,
    ) {
        let forecast = |target_id: EntityId| {
            let target = self.world.entities.get(target_id).unwrap_or_else(|| {
                panic!(
                    "NPC {} requires a destination forecast for missing actor {}",
                    npc_id.index(),
                    target_id.index()
                )
            });
            let mut input = extract_forecast_input(target).unwrap_or_else(|| {
                panic!(
                    "NPC {} requires a destination forecast for non-actor {}",
                    npc_id.index(),
                    target_id.index()
                )
            });
            if let Some(positions) = positions_before_movement {
                let position = self.position_at_owner_boundary(target_id, npc_id, positions, true);
                input.position_map_x = position.x;
                input.position_map_y = position.y;
            }
            crate::ai::prepare_forecast_destination_for_ia(
                &input,
                self.script_domains.interactables.doors.as_slice(),
                &self.world.fast_grid.level.sectors,
                &self.world.fast_grid.level.sector_number_map,
            )
        };

        let (primary, missed) = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::enemy_ai)
            .map(|ai| (ai.base.primary_target, ai.missed_pc))
            .unwrap_or((0, 0));
        tick_data.enemy_detectable_forecasts.clear();
        let enemy_handles = self
            .world
            .entities
            .get(npc_id)
            .and_then(Entity::npc_data)
            .map(|npc| {
                npc.detectable_lists[crate::element::DetectableType::Enemy as usize]
                    .iter()
                    .filter_map(|detectable| detectable.element)
                    .map(|entity_id| entity_id.index())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for handle in enemy_handles {
            let target_id = self.entity_id_for_index(handle).unwrap_or_else(|| {
                panic!(
                    "NPC {} has Enemy detectable for missing actor {}",
                    npc_id.index(),
                    handle
                )
            });
            tick_data
                .enemy_detectable_forecasts
                .push((handle, forecast(target_id)));
        }
        if tick_data.primary_target_is_pc && primary != 0 {
            let target_id = self.entity_id_for_index(primary).unwrap_or_else(|| {
                panic!(
                    "NPC {} has missing primary-target actor {}",
                    npc_id.index(),
                    primary
                )
            });
            tick_data.primary_target_forecast = Some(forecast(target_id));
        }
        if tick_data.missed_pc_is_pc && missed != 0 {
            let target_id = self.entity_id_for_index(missed).unwrap_or_else(|| {
                panic!(
                    "NPC {} has missing missed-PC actor {}",
                    npc_id.index(),
                    missed
                )
            });
            tick_data.missed_pc_forecast = Some(forecast(target_id));
        }
        for soldier in &mut tick_data.camp_soldiers {
            // Only officers are ever selected as forecasted destinations by
            // AlertOfficer/reporting paths. Ordinary camp soldiers remain
            // live metadata inputs but must not consume building-exit RNG
            // merely because this owner entered Think.
            if soldier.rank != crate::profiles::ProfileRank::Officer {
                continue;
            }
            let target_id = self.entity_id_for_index(soldier.handle).unwrap_or_else(|| {
                panic!(
                    "NPC {} has missing camp-soldier actor {}",
                    npc_id.index(),
                    soldier.handle
                )
            });
            soldier.forecast_destination = Some(forecast(target_id));
        }
    }

    pub(super) fn apply_owner_relative_tick_positions(
        &self,
        npc_id: EntityId,
        target_id: Option<EntityId>,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
        tick_data: &mut AiPerTickData,
    ) {
        if let Some(target_id) = target_id {
            let position =
                self.position_at_owner_boundary(target_id, npc_id, positions_before_movement, true);
            if let Some(target) = &mut tick_data.primary_target_position {
                target.x = position.x;
                target.y = position.y;
            }
        }
        for fighter in &mut tick_data.nearby_fighters {
            if let Some(id) = self.entity_id_for_index(fighter.handle) {
                let position =
                    self.position_at_owner_boundary(id, npc_id, positions_before_movement, true);
                fighter.position.x = position.x;
                fighter.position.y = position.y;
            }
        }
        for soldier in &mut tick_data.camp_soldiers {
            let id = self.entity_id_for_index(soldier.handle).unwrap_or_else(|| {
                panic!(
                    "NPC {} has missing camp soldier {} at its owner boundary",
                    npc_id.index(),
                    soldier.handle
                )
            });
            let position =
                self.position_at_owner_boundary(id, npc_id, positions_before_movement, true);
            soldier.position.x = position.x;
            soldier.position.y = position.y;
        }
        self.prepare_detection_forecasts_for_owner(
            npc_id,
            Some(positions_before_movement),
            tick_data,
        );
    }

    /// Test seam for creation-slot parity: capture the ordinary tick-start AI
    /// view, mutate live entity/sequence state, then run only RefreshDetection.
    #[cfg(test)]
    pub(crate) fn refresh_detection_after_world_snapshot_for_test(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        mutate_live_state: impl FnOnce(&mut Self),
    ) {
        let world = self.tick_enemy_ai_build_world_view(assets, None);
        mutate_live_state(self);
        self.tick_enemy_ai_refresh_detection(sim, assets, &world, None, None, false);
    }

    #[cfg(test)]
    pub(crate) fn enemy_optical_viewer_context_for_test(&self, npc_id: EntityId) -> bool {
        let entity = self
            .world
            .entities
            .get(npc_id)
            .expect("test Enemy optical viewer should exist");
        let building_sector = self.entity_building_sector(entity.element_data().sector());
        SoldierSightContext::from_npc_viewer(npc_id, entity, building_sector).is_some()
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
        enemy_targets: &[EnemyOpticalTarget],
        universal_frame: u32,
        golden_eye: bool,
        is_forest_level: bool,
        view_radius_cache: &OwnerViewRadiusCache,
    ) -> Option<(Vec<crate::ai::Stimulus>, AiPerTickData)> {
        use crate::ai::AiState;
        use crate::element::{ActionState, Posture};

        let pc_snapshots = world.pcs.as_slice();
        let soldier_snapshots = world.soldiers.as_slice();
        let unconscious_soldiers = world.unconscious_soldiers.as_slice();
        let primary_target_multiplicity =
            self.ai.global.primary_target_multiplicity_scratch.clone();
        let detection_target_multiplicity = &world.detection_target_multiplicity;
        let npc_jump_lines = &world.npc_jump_lines;

        // -- Read NPC state in a scoped borrow --
        let (viewer, viewer_inside_building) = {
            let entity = self.world.entities.get(npc_id)?;
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let inside_building = self.entity_data_inside_building(entity.element_data());
            (
                SoldierSightContext::from_npc_viewer(npc_id, entity, building_sector)?,
                inside_building,
            )
        };
        let eye = viewer.eye;
        let eye_world = viewer.eye_world;
        let dir = viewer.dir;
        let layer = viewer.layer;
        let view_radius = viewer.view_radius;
        let eye_status = viewer.eye_status;
        let current_state = viewer.current_state;
        let view_forward = viewer.view_forward;
        let real_half_aperture = viewer.real_half_aperture;
        let npc_posture = viewer.posture;
        let entity_sector = viewer.sector;
        let viewer_blipped = viewer.blipped;
        let me_ground_position = viewer.ground_position;
        // Silence the "unused" warning on the `_action_state` slot
        // we keep for readability of the destructure pattern.
        let _ = ActionState::Waiting;

        // Resolve the viewer's building sector from the entity's
        // cached sector (set during door-pass transitions).  Used by
        // RefreshDetection / IsDetecting to short-circuit visibility
        // when the viewer is indoors.
        let viewer_building_sector = self.entity_building_sector(entity_sector);

        let is_night_or_fog = matches!(
            self.world.weather.ambiance,
            crate::engine::types::Ambiance::Night | crate::engine::types::Ambiance::Fog
        );
        // Per-NPC frame-counter phase offset so not every NPC re-runs
        // detection on the same tick. The Original keys this with the
        // entity's creation order, not its current marrayElements slot.
        let modified_frame = refresh_detection_modified_frame(
            universal_frame,
            self.original_static_creation_order(npc_id),
        );
        let alert_status = viewer.alert_status;
        // Lacklandist ComputeVisibility lets Stare / Follow and non-Green
        // alert status bypass the per-entry cadence. Original's Royalist arm
        // never consults this flag and remains strictly modulo-16.
        let lacklandist_refresh_always =
            lacklandist_visibility_refresh_always(eye_status, alert_status);
        // InstantDetection is camp-wide in the Original: Royalists always
        // commit Enemy sightings, while Lacklandists accumulate in the
        // sleeping/default/wondering states.
        let instant_detection = viewer.camp == Camp::Royalists
            || !matches!(
                current_state,
                AiState::Sleeping | AiState::Default | AiState::Wondering
            );

        // -- Mutating pass: update detectable list + suspects --
        // `&self.sight_obstacles` and `self.world.entities.get_mut(...)`
        // are disjoint fields on `self`, so the split borrow is
        // valid.
        let mut think_tick_data: Option<AiPerTickData> = Some(AiPerTickData::stub());
        let mut enemy_stimuli: Vec<crate::ai::Stimulus> = Vec::new();
        let mut reveal_targets: Vec<EntityId> = Vec::new();
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
            let entity = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                panic!(
                    "NPC {} disappeared during its Enemy optical scan",
                    npc_id.index()
                )
            });
            let npc = entity.npc_data_mut().unwrap_or_else(|| {
                panic!(
                    "Enemy optical observer {} has no required NPC state",
                    npc_id.index()
                )
            });

            // Beggar-trick learning.  Capture the AI's current
            // `got_the_beggar_trick` flag before taking a mut borrow
            // on `detectable_lists` (both fields live under
            // `soldier.npc`).  We mutate a local during the loop and
            // write back after the borrow on `detectables` releases.
            let mut got_beggar_trick = npc
                .ai_brain
                .base()
                .map(|ai| ai.got_the_beggar_trick)
                .unwrap_or_else(|| {
                    panic!(
                        "Enemy optical observer {} has no required AI controller",
                        npc_id.index()
                    )
                });

            let enemy_idx = DetectableType::Enemy as usize;
            let detectables: &mut Vec<Detectable> = &mut npc.detectable_lists[enemy_idx];

            // Original CleanUpDetectables removes dead enemies only. The
            // AddDetectable policy governs new entries, but an existing entry
            // can outlive a later camp/role change and remains authoritative.
            // Missing or non-human targets still indicate corrupted NPC
            // state and must fail with observer/target context.
            let before = detectables.len();
            detectables.retain(|d| {
                let target_id = d.element.unwrap_or_else(|| {
                    panic!(
                        "Enemy detectable for NPC {} has no target handle",
                        npc_id.index()
                    )
                });
                let target = enemy_targets
                    .iter()
                    .find(|target| target.id == target_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "Enemy detectable target {} for NPC {} is missing or is not a PC/soldier",
                            target_id.index(),
                            npc_id.index()
                        )
                    });
                !target.dead
            });
            if before != detectables.len() {
                tracing::trace!(
                    npc = ?npc_id,
                    before,
                    after = detectables.len(),
                    "CleanUpDetectables removed dead Enemy entries"
                );
            }
            tracing::trace!(
                npc = ?npc_id,
                camp = ?viewer.camp,
                ?alert_status,
                ?eye_status,
                lacklandist_refresh_always,
                entries = ?detectables.iter().map(|d| d.element).collect::<Vec<_>>(),
                "Enemy detectable list"
            );

            // Per-target visibility pass.
            //
            // `best_target` tracks the unoccupied-preferred primary
            // target pick — lowest-score wins, where score is the
            // Euclidean distance + a penalty for how many friendly
            // soldiers already target this PC.  We use `u32::MAX`
            // for "no target yet" so the first visible PC always
            // replaces it.
            let mut sum_sharpness_new: u16 = 0;
            let mut best_target: Option<(EntityId, MapPoint, u32)> = None;
            let mut max_sharpness: u32 = 0;
            let viewer_in_building = viewer_building_sector.is_some();
            // Sharpness depends only on the observer's posture. Keep this in
            // the scan scope so the predetection diagnostic below reports the
            // exact same conversion used for every detectable.
            let view_speed = if npc_posture == Posture::LeaningOut {
                ai_vision::LOOK_DOWN_BASE_VIEW_SPEED
            } else {
                ai_vision::BASE_VIEW_SPEED
            };

            for det in detectables.iter_mut() {
                let target_id = det
                    .element
                    .expect("enemy detectable survived cleanup without a target entity handle");
                let target = enemy_targets
                    .iter()
                    .find(|target| target.id == target_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "Enemy detectable target {} for NPC {} vanished after cleanup",
                            target_id.index(),
                            npc_id.index()
                        )
                    });

                // Original's outer RefreshDetection box gate precedes
                // ComputeVisibility and its cadence. A previously visible
                // target and every target while indoors still enter; all
                // others must lie in the GetPositionGround world-X/Y
                // radius/aspect bounding box.
                if !refresh_detection_scans_target(
                    det.last_visibility,
                    viewer_inside_building,
                    me_ground_position,
                    view_radius,
                    target.ground_position,
                ) {
                    tracing::trace!(
                        observer = ?npc_id,
                        target = ?target_id,
                        view_radius,
                        viewer_x = me_ground_position.x,
                        viewer_y = me_ground_position.y,
                        target_x = target.ground_position.x,
                        target_y = target.ground_position.y,
                        "Enemy detectable outside RefreshDetection box"
                    );
                    det.seen_now = false;
                    det.last_visibility = 0.0;
                    continue;
                }

                // ComputeVisibility returns zero for blind eyes before either
                // camp's cadence branch. Clear the cached sample too so a
                // closed cadence cannot resurrect a formerly visible target.
                if eye_status.is_blind() {
                    det.seen_now = false;
                    det.last_visibility = 0.0;
                    continue;
                }
                // Do not reject a target merely because its logical movement
                // layer differs. Original `ComputeVisibility(human)` compares
                // the full 3D eye/detection points and lets `IsDetecting`
                // decide line of sight; actors on visible stairs, roofs, and
                // adjoining elevations can therefore be seen cross-layer.
                // Original Lacklandist ComputeVisibility rejects HollowMan
                // targets before its PC-vs-soldier cadence branch. Keep the
                // detectable (CleanUpDetectables only removes dead enemies),
                // but clear both live visibility and the cached sample.
                if viewer.camp == Camp::Lacklandists && target.hollow_man {
                    det.seen_now = false;
                    det.last_visibility = 0.0;
                    continue;
                }

                // Original's Lacklandist PC-only blip and guard gates run
                // before the PC cadence decision. They invalidate the cached
                // sample even when this frame would otherwise reuse it.
                if viewer.camp == Camp::Lacklandists
                    && target.is_pc
                    && viewer_blipped
                    && !viewer_inside_building
                {
                    det.seen_now = false;
                    det.last_visibility = 0.0;
                    continue;
                }
                if viewer.camp == Camp::Lacklandists
                    && target.is_pc
                    && !det.seen_last_frame
                    && target.guarded
                {
                    det.seen_now = false;
                    det.last_visibility = 0.0;
                    continue;
                }

                let frequency = if target.is_soldier || viewer.camp == Camp::Royalists {
                    ai_vision::DETECTION_FREQUENCY_ENEMY_NPC
                } else {
                    ai_vision::DETECTION_FREQUENCY_ENEMY_PC
                };
                let gate_open = modified_frame.is_multiple_of(frequency)
                    || (viewer.camp == Camp::Lacklandists && lacklandist_refresh_always);
                tracing::trace!(
                    observer = ?npc_id,
                    target = ?target_id,
                    modified_frame,
                    frequency,
                    gate_open,
                    camp = ?viewer.camp,
                    lacklandist_refresh_always,
                    "Enemy detection cadence gate"
                );

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
                    let target_in_same_building =
                        viewer_in_building && viewer_building_sector == target.building_sector;
                    // Posture-based Z offsets for the 3D close-range
                    // distance check (see
                    // `ai_vision::compute_visibility`).  The LOS
                    // raycast itself is still 2D until sight-obstacle
                    // data carries Z.
                    //
                    let target_obstacle_handle = target.obstacle_idx;
                    let target_obstacle = target_obstacle_handle.map(|handle| {
                        sight_obstacles.get(usize::from(handle)).unwrap_or_else(|| {
                            panic!(
                                "Enemy visibility target {} requires missing obstacle {}",
                                target_id.index(),
                                u16::from(handle)
                            )
                        })
                    });
                    let q = ai_vision::VisibilityQuery {
                        viewer_los: eye,
                        viewer_world: eye_world,
                        viewer_direction: dir,
                        view_forward,
                        view_radius,
                        viewer_eye_status: eye_status,
                        real_half_aperture,
                        viewer_in_building,
                        target_in_same_building,
                        forest_180_degree_view: forest_180_degree_view_enabled(
                            is_forest_level,
                            viewer.camp,
                        ),
                        golden_eye_mode: golden_eye,
                        // Resolved lazily below at Original's
                        // ComputeViewRadius call site.
                        effective_view_radius: view_radius as f32,
                        target_is_active_and_outside_building: target.active
                            && target.building_sector.is_none(),
                        target_los: crate::stealth::detection_point_xy(
                            target.position,
                            target.posture,
                            target.direction,
                        ),
                        target_world: target.detection_point.unwrap_or_else(|| {
                            panic!(
                                "live Enemy target {} for NPC {} has no detection point",
                                target_id.index(),
                                npc_id.index()
                            )
                        }),
                        target_posture: target.posture,
                        target_action_state: target.action_state,
                        target_is_pc: target.is_pc,
                        sight_obstacles,
                        fast_grid: &self.world.fast_grid,
                        layer,
                        target_unconscious: target.unconscious,
                        target_passing_door: target.passing_door,
                    };
                    let effective_view_radius = std::cell::Cell::new(None);
                    let visibility =
                        ai_vision::compute_visibility_with_effective_radius(&q, || {
                            let radius =
                                view_radius_cache.get_or_compute(target_obstacle_handle, || {
                                    ai_vision::compute_view_radius(
                                        q.viewer_world,
                                        view_radius,
                                        view_forward,
                                        real_half_aperture,
                                        is_night_or_fog,
                                        &self.world.fast_grid,
                                        sight_obstacles,
                                        target_obstacle,
                                    )
                                });
                            effective_view_radius.set(Some(radius));
                            radius
                        });
                    tracing::trace!(
                        observer = ?npc_id,
                        target = ?target_id,
                        modified_frame,
                        effective_view_radius = ?effective_view_radius.get(),
                        visibility,
                        viewer_x = q.viewer_world.x,
                        viewer_y = q.viewer_world.y,
                        viewer_z = q.viewer_world.z,
                        target_x = q.target_world.x,
                        target_y = q.target_world.y,
                        target_z = q.target_world.z,
                        "Enemy optical visibility refresh"
                    );
                    visibility
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
                    let detection_speed_factor =
                        if target.is_pc && viewer.camp == Camp::Lacklandists {
                            let detection_speed_pct = if is_forest_level {
                                target.detection_speed_in_forest
                            } else {
                                target.detection_speed_in_city
                            };
                            0.01 * detection_speed_pct as f32
                        } else {
                            1.0
                        };
                    frequency as f32 * visibility_raw * detection_speed_factor
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
                visibility = apply_enemy_beggar_disguise(
                    viewer.camp,
                    target.is_pc,
                    &mut got_beggar_trick,
                    target.order_type,
                    visibility,
                );

                // Sharpness depends on posture.  Leaning out uses
                // 10x faster detection (200 vs 20).
                let sharpness = detection_sharpness(view_speed, visibility);
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
                    target_x = target.position.x,
                    target_y = target.position.y,
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
                    sum_sharpness_new =
                        accumulate_detection_sharpness(sum_sharpness_new, sharpness);
                }

                if is_visible {
                    // Unoccupied-preferred primary-target scoring:
                    //   distance = Distance(enemy)
                    //   distance += 100 * primary_target_multiplicity
                    //   pick the lowest distance
                    let dx = target.position.x - eye.x;
                    let dy = target.position.y - eye.y;
                    let dist_sq = dx * dx + dy * dy;
                    let dist = dist_sq.sqrt() as u32;
                    let mult = detection_target_multiplicity
                        .get(&target_id)
                        .copied()
                        .unwrap_or(0);
                    let score = dist + 100 * mult;
                    let replace = match best_target {
                        None => true,
                        Some((_, _, s)) => score < s,
                    };
                    if replace {
                        best_target = Some((target_id, target.position, score));
                    }
                }

                // Single-field update.  Next frame's edge-trigger
                // reads this value directly.
                det.seen_now = is_visible;
                // Original's outer RefreshDetection loop writes the final
                // wrapper result on every scanned entry. Eligible closed
                // cadence reuses the same value, while the beggar-disguise
                // post-filter must be able to replace that cached value by 0.
                det.last_visibility = visibility;
                // Original updates `muwMaximalVisibility` from the integer
                // sharpness returned after ComputeVisibility has reused a
                // detectable's cached visibility on closed-cadence frames.
                // Using `visibility_raw` here would falsely report zero every
                // other frame and can end a shadow investigation early.
                max_sharpness = max_sharpness.max(u32::from(sharpness));
            }

            // Write back the beggar-trick flag if a mid-transition
            // sighting flipped it during the loop.
            if got_beggar_trick
                && let Some(ai) = npc.ai_brain.base_mut()
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

            // `muwMaximalVisibility` belongs to RHElementActorNPC, not to
            // malignity AI. Every NPC observer, including civilians backed by
            // FriendlyAi, publishes the Enemy-bucket maximum before the later
            // detectable-type buckets fold in their own sharpness.
            if let Some(ai) = npc.ai_brain.base_mut() {
                ai.max_visibility = max_sharpness;
            }

            let my_camp = viewer.camp;
            if viewer.camp == Camp::Lacklandists
                && let Some(enemy_ai) = npc.ai_brain.enemy_mut()
            {
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
                    owner_live_position: Some(viewer.position),
                    // Prepared without RNG only after this scan produces an
                    // Enemy stimulus block.
                    primary_target_forecast: None,
                    primary_target_is_pc: pc_snapshots
                        .iter()
                        .any(|pc| pc.id.index() == enemy_ai.base.primary_target),
                    missed_pc_forecast: None,
                    missed_pc_is_pc: pc_snapshots
                        .iter()
                        .any(|pc| pc.id.index() == enemy_ai.missed_pc),
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
                tick_data.enemy_detectable_positions = enemy_targets
                    .iter()
                    .map(|target| {
                        (
                            target.id.index(),
                            crate::ai::Position {
                                x: target.planning_position.x,
                                y: target.planning_position.y,
                                sector: target.sector,
                                level: target.layer,
                            },
                        )
                    })
                    .collect();
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
                for det in npc.detectable_lists[enemy_idx].iter() {
                    if det.seen_last_frame
                        && let Some(elem) = det.element
                    {
                        tick_data.seen_last_frame_enemies.push(elem.index());
                    }
                }
                for det in npc.detectable_lists[enemy_idx].iter() {
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
                // Preserve every unconscious, non-carried enemy candidate in
                // fighter-registry order. The final BattleDecisions fallback
                // owns the observable IsDetecting360Degrees query; snapshot
                // construction must not issue or cache LOS speculatively.
                //
                // Scoped to PCs here — unconscious enemy NPCs
                // would require iterating the opposing-camp
                // soldier list.  In practice only the player's
                // merry men can knock soldiers out, and the
                // battle path already prefers standing targets,
                // so the scan rarely matters.  Extending to
                // enemy-camp `soldier_snapshots` would duplicate
                // this loop with an additional camp filter.
                for pc in pc_snapshots {
                    if !pc.unconscious || pc.carried {
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

                // Precompute the nearby-friend facts consumed by
                // BattleDecisions.  Do not update AiBase::list_us here:
                // Original only rebuilds its persistent mlistUs at the
                // specific AI routines that own that list (not during the
                // per-frame detection snapshot).
                const US_LIST_SQ_RADIUS: f32 = 500.0 * 500.0;
                let my_company = enemy_ai.company_number;
                let my_pride = enemy_ai.soldier_profile_pride;
                tick_data.friends_lower_company = 0;
                tick_data.soldiers_lower_pride = false;
                // MakeBattlePredecisions: self contributes 100 + own pride.
                tick_data.us_battle_points = 100 + my_pride as u32;
                tick_data.has_officer_nearby = false;
                tick_data.simple_soldiers_near = false;
                tick_data.friends_nearer_to_enemy = 0;

                // Also add visible PCs to us-list (they fight on our
                // side when the NPC is Royalist, but for Lacklandists
                // PCs are enemies — skip). For now, only add NPCs.
                for ss in soldier_snapshots {
                    if ss.id == npc_id || ss.camp != my_camp {
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
                }

                // Primary target multiplicity
                tick_data.primary_target_multiplicity.clear();
                for (&target, &mult) in &primary_target_multiplicity {
                    tick_data.primary_target_multiplicity.push((target, mult));
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
                }

                // ── Camp soldier snapshots for alert functions ──
                // Provides alert_officer / alert_soldiers with a view
                // of all same-camp soldiers (any distance).  The alert
                // functions do their own distance filtering.
                tick_data.camp_soldiers.clear();
                tick_data.camp_unconscious_soldiers.clear();
                for (ko_id, ko_camp, knocked_out_in_money_fight) in unconscious_soldiers {
                    if *ko_id == npc_id || *ko_camp != my_camp {
                        continue;
                    }
                    tick_data.camp_unconscious_soldiers.push(
                        crate::ai_enemy::CampUnconsciousSoldierInfo {
                            handle: ko_id.index(),
                            knocked_out_in_money_fight: *knocked_out_in_money_fight,
                        },
                    );
                }
                // Visibility between the owner and these soldiers is
                // intentionally not part of the snapshot. Original queries
                // it only inside BattleDecisions, CommandSoldiersToAttack,
                // and MaybeOfficerSeesMeFighting; eager LOS here would fire
                // O(N²) raycasts and perturb the cache on idle ticks.
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
                    tick_data
                        .camp_soldiers
                        .push(crate::ai_enemy::CampSoldierInfo {
                            handle: ss.id.index(),
                            active: ss.active,
                            position: ss_position,
                            position_world: ss.position_world,
                            direction: ss.direction,
                            rank: ss.rank,
                            ai_state: ss.ai_state,
                            ai_substate: ss.ai_substate,
                            is_able_to_fight: ss.able_to_fight,
                            is_dead: ss.is_dead,
                            primary_target: ss.primary_target,
                            pride: ss.pride,
                            is_able_to_help: ss.able_to_help,
                            script_locked: ss.script_locked,
                            ai_lock_frozen: ss.ai_lock_frozen,
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
                            forecast_destination: ss.forecast_destination.clone(),
                            detectable_bodies: ss.detectable_bodies.clone(),
                            seek_position: ss.ai_seek_position,
                            current_task_priority: ss.current_task_priority,
                            minimal_task_priority: ss.minimal_task_priority,
                            view_direction: ss.view_direction,
                            view_radius: ss.view_radius,
                            real_half_aperture: ss.real_half_aperture,
                            eye_blind: ss.eye_blind,
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
                            elevation: f32::from(me_snap.elevation),
                        });
                    }

                    // Friendly soldiers from the same-camp fighter
                    // registry (excluding self). Original inserts self first,
                    // which makes FillListWithAllNearFighters require every
                    // additional same-camp fighter to be swordfighting.
                    // ReconsiderSwordfightObservation rebuilds the
                    // us-list by scanning all nearby same-camp
                    // fighters every time; using the previous Rust
                    // `list_us` here made this snapshot stale and
                    // let multiple observers miss a friend already
                    // walking / running / charging the same target.
                    for ss in soldier_snapshots {
                        if ss.id.index() == me_handle
                            || ss.camp != my_camp
                            || !ss.able_to_fight
                            || !ss.is_swordfighting
                        {
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
                            elevation: f32::from(ss.elevation),
                        });
                    }

                    // Hostile PCs from the global fighter registry. Original
                    // FAST_OVERVIEW rebuilds mlistThem from every nearby
                    // enemy-camp fighter; it does not use the NPC's prior
                    // detection list.
                    for pc in pc_snapshots {
                        if !pc.able_to_fight {
                            continue;
                        }
                        let enemy_handle = pc.id.index();
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
                            elevation: f32::from(pc.ground_elevation),
                        });
                    }
                }
                think_tick_data = Some(tick_data);
            }

            // Running worst-detected-type (smallest enum value
            // wins).  We only drive Enemy detection here right now,
            // so the guard collapses to "promote from None / higher
            // to Enemy on any fresh sharpness this frame".  Body /
            // Object arms apply the same check when they are
            // ported.
            if sum_sharpness_new > 0
                && (npc.worst_detected_type as u32) > (DetectableType::Enemy as u32)
            {
                npc.worst_detected_type = DetectableType::Enemy;
            }

            // ── Pre-detection shadow event ────────────────────
            // Per-detectable edge-triggered EVENT_SEES_SHADOW on the
            // rising edge of
            //   shadow_is_seen = (sharpness > 0)
            //                 && suspects_before_scan[type]
            //                    >= SHADOW_DETECTION_THRESHOLD
            // No outer `instant_detection` / upper-bound guards.
            // Each detectable dispatches its own event on its own
            // rising edge, so no `break` after the first one.
            //
            // HandlePredetection runs before this frame's sharpness is added
            // to the suspect accumulator. It also returns without touching
            // the latch for non-PC and guarded-PC targets.
            let suspects_before_scan = npc.detection_suspects[enemy_idx];
            for det in npc.detectable_lists[enemy_idx].iter_mut() {
                if let Some(target_id) = det.element {
                    let target = enemy_targets
                        .iter()
                        .find(|target| target.id == target_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "shadow Enemy target {} for NPC {} is missing from the live optical view",
                                target_id.index(),
                                npc_id.index()
                            )
                        });
                    let shadow_seen_before = det.shadow_seen_last_frame;
                    let queued = update_predetection_shadow_latch(
                        det.seen_now,
                        suspects_before_scan,
                        target.is_pc,
                        target.guarded,
                        &mut det.shadow_seen_last_frame,
                    );
                    tracing::trace!(
                        target: "shadow_predetection",
                        frame = universal_frame,
                        observer = ?npc_id,
                        observer_index = npc_id.index(),
                        detectable_type = ?DetectableType::Enemy,
                        detectable_target = ?target_id,
                        detectable_target_index = target_id.index(),
                        sharpness = u32::from(detection_sharpness(
                            view_speed,
                            det.last_visibility,
                        )),
                        suspects_before = suspects_before_scan,
                        is_pc = target.is_pc,
                        guarded = target.guarded,
                        seen_now = det.seen_now,
                        seen_last_frame = det.seen_last_frame,
                        shadow_seen_now = det.shadow_seen_now,
                        shadow_seen_before,
                        shadow_seen_after = det.shadow_seen_last_frame,
                        last_visibility = det.last_visibility,
                        queued,
                        "evaluated Enemy shadow predetection edge"
                    );
                    if !queued {
                        continue;
                    }
                    // Queue EVENT_SEES_SHADOW for this NPC's post-detection
                    // FIFO drain, ahead of its Enemy VIEW / OUTOFVIEW block.
                    let shadow_pos = crate::ai::Position {
                        x: target.position.x,
                        y: target.position.y,
                        sector: target.sector,
                        level: target.layer,
                    };
                    let stimulus = crate::ai::Stimulus::with_position(
                        crate::ai::StimulusType::EventSeesShadow,
                        shadow_pos,
                    );
                    if let Some(ai) = npc.ai_brain.base_mut() {
                        ai.outbox.detection.stimuli.push(stimulus);
                    }
                }
            }

            // Original adds the current scan only after every detectable has
            // run HandlePredetection against the prior accumulator.
            let suspects = &mut npc.detection_suspects[enemy_idx];
            *suspects = suspects.wrapping_add(sum_sharpness_new);

            // Commit condition.
            let threshold_hit = *suspects as u32 >= ai_vision::DETECTION_SUSPECT_THRESHOLD;
            let instant_hit = instant_detection && sum_sharpness_new > 0;

            if threshold_hit || instant_hit {
                // Reset suspects on commit.
                *suspects = 0;
            } else {
                *suspects = cool_detection_suspect(sum_sharpness_new, *suspects, universal_frame);
            }

            // Seed the frame maximum from Enemy. Body and Object fold their
            // persistent suspects into it below; only after every type has
            // run may the remembered worst type be cleared.
            npc.maximal_detection_suspect = npc.detection_suspects[enemy_idx];

            // Walk every detectable and edge-detect `seen_last_frame`.
            //   - Rising edge (detected && !latched) fires EVENT_VIEW for
            //     every Enemy detectable in list order.
            //   - Falling edge (!detected && latched) fires
            //     EVENT_OUTOFVIEW and clears the latch.
            // On commit frames both edges run; on non-commit frames
            // we still run the falling-edge check so NPCs react to
            // lost sight the instant it happens.
            let committed = threshold_hit || instant_hit;
            for det in npc.detectable_lists[enemy_idx].iter_mut() {
                let was_seen = det.seen_last_frame;
                let is_seen = det.seen_now;
                let falling_edge = !is_seen && was_seen;
                // HandleDetection's second pass intersperses rising VIEW and
                // falling OUTOFVIEW by detectable-list order.
                if committed && is_seen && !was_seen {
                    let target_id = det.element.unwrap_or_else(|| {
                        panic!(
                            "rising Enemy detectable for NPC {} has no target",
                            npc_id.index()
                        )
                    });
                    let target = enemy_targets
                        .iter()
                        .find(|target| target.id == target_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "rising Enemy target {} for NPC {} is missing from the live optical view",
                                target_id.index(),
                                npc_id.index()
                            )
                        });

                    // Enemy-bucket detection always emits EVENT_VIEW. A
                    // disguised PC that has not been seen through has zero
                    // visibility earlier in the scan; EVENT_SEES_BEGGAR is
                    // exclusive to the separate Beggar detectable bucket.
                    enemy_stimuli.push(crate::ai::Stimulus::with_human(
                        crate::ai::StimulusType::EventView,
                        target_id.index(),
                    ));
                    if viewer.camp == Camp::Royalists && target.is_soldier && target.blipped {
                        reveal_targets.push(target_id);
                    }
                }
                if falling_edge && let Some(target_id) = det.element {
                    enemy_stimuli.push(crate::ai::Stimulus::with_human(
                        crate::ai::StimulusType::EventOutOfView,
                        target_id.index(),
                    ));
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

            // The detection-built tick input is assembled before the latch
            // walk to avoid conflicting AI/list borrows. Refresh its latch
            // snapshot now so every queued Think observes the final state
            // produced by HandleDetection, including every rising VIEW.
            if let Some(tick_data) = think_tick_data.as_mut() {
                tick_data.seen_last_frame_enemies.clear();
                tick_data.seen_last_frame_enemies.extend(
                    npc.detectable_lists[enemy_idx]
                        .iter()
                        .filter(|det| det.seen_last_frame)
                        .filter_map(|det| det.element.map(EntityId::index)),
                );
            }
        }

        // HandleDetection reveals newly seen blipped NPCs inline, after the
        // complete scan has built its FIFO but before the first queued Think.
        for target_id in reveal_targets {
            let target = self.world.entities.get_mut(target_id).unwrap_or_else(|| {
                panic!(
                    "rising Enemy target {} for NPC {} disappeared before RevealBlip",
                    target_id.index(),
                    npc_id.index()
                )
            });
            target.reveal_blip();
        }

        match (enemy_stimuli.is_empty(), think_tick_data) {
            (false, Some(tick_data)) => Some((enemy_stimuli, tick_data)),
            (true, _) => None,
            (false, None) => {
                panic!("detection queued Enemy Think stimuli without per-tick enemy input")
            }
        }
    }

    /// Build every PC/soldier that may legally occupy an Enemy list in global
    /// creation order. The actual scan walks the NPC's live detectable list;
    /// this view only supplies target fields without aliasing the observer.
    fn tick_enemy_ai_build_live_enemy_optical_targets(
        &self,
        world: &AiWorldView,
    ) -> Vec<EnemyOpticalTarget> {
        self.world
            .entities
            .humans()
            .filter_map(|(id, entity)| match entity {
                Entity::Pc(pc) => {
                    let entity_id: EntityId = id.into();
                    let dead = pc.pc.life_points <= 0;
                    let snapshot = world
                        .pcs
                        .iter()
                        .find(|snapshot| snapshot.id == entity_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "PC {} is absent from the owner-relative Enemy optical snapshot",
                                entity_id.index()
                            )
                        });
                    let posture = pc.element.posture;
                    let ground_z = pc.element.position().z;
                    let order_type = self
                        .orders
                        .sequence_manager
                        .current_order_for_actor(entity_id)
                        .map(|(_, _, order)| order.order_type)
                        .unwrap_or(crate::order::OrderType::Invalid);
                    Some(EnemyOpticalTarget {
                        id: entity_id,
                        position: snapshot.position,
                        planning_position: MapPoint::from_world_xyz(
                            snapshot.position_world.x,
                            snapshot.position_world.y,
                            snapshot.position_world.z,
                        ),
                        ground_position: GroundPoint::from_map_and_z(
                            snapshot.position,
                            ground_z,
                        ),
                        sector: pc.element.sector(),
                        layer: pc.element.layer(),
                        posture,
                        action_state: pc.actor.action_state,
                        building_sector: self.entity_building_sector(pc.element.sector()),
                        // Original ComputeDetectionPoint's default posture
                        // arm leaves its already-initialized GetPosition()
                        // result unchanged. A living loaded PC with
                        // RHPOSTURE_UNDEFINED therefore has a valid
                        // zero-offset detection point.
                        eye_z: (!dead).then(|| {
                            ground_z + crate::stealth::detection_z_for_posture(posture, false)
                        }),
                        detection_point: (!dead).then(|| {
                            crate::stealth::detection_point_world(
                                snapshot.position_world,
                                posture,
                                pc.element.direction(),
                                false,
                            )
                        }),
                        ground_z,
                        direction: pc.element.direction(),
                        active: pc.element.active,
                        unconscious: pc.human.unconscious,
                        passing_door: pc.actor.active_door_pass.is_some(),
                        obstacle_idx: pc.element.obstacle_index(),
                        is_pc: true,
                        is_soldier: false,
                        dead,
                        hollow_man: pc.human.hollow_man,
                        guarded: pc.pc.guard.is_some(),
                        detection_speed_in_forest: snapshot.detection_speed_in_forest,
                        detection_speed_in_city: snapshot.detection_speed_in_city,
                        order_type,
                        blipped: pc.element.blipped,
                    })
                }
                Entity::Soldier(soldier) => {
                    let entity_id: EntityId = id.into();
                    let posture = soldier.element.posture;
                    let is_rider = soldier.soldier.rider;
                    let dead = soldier.npc.life_points <= 0;
                    let (position, position_world) = world
                        .soldiers
                        .iter()
                        .find(|snapshot| snapshot.id == entity_id)
                        .map(|snapshot| (snapshot.position, snapshot.position_world))
                        .unwrap_or_else(|| {
                            assert!(
                                dead || !soldier.element.active || soldier.human.unconscious,
                                "active conscious soldier {} is absent from the owner-relative Enemy optical snapshot",
                                entity_id.index()
                            );
                            // Inactive, unconscious, and dead soldiers do not
                            // participate in the globally batched movement
                            // slice, so their live position is also their
                            // owner-boundary position.
                            (
                                soldier.element.position_map(),
                                soldier.element.position(),
                            )
                        });
                    Some(EnemyOpticalTarget {
                        id: entity_id,
                        position,
                        planning_position: MapPoint::from_world_xyz(
                            position_world.x,
                            position_world.y,
                            position_world.z,
                        ),
                        ground_position: GroundPoint::from_map_and_z(
                            position,
                            soldier.element.position().z,
                        ),
                        sector: soldier.element.sector(),
                        layer: soldier.element.layer(),
                        posture,
                        action_state: soldier.actor.action_state,
                        building_sector: self.entity_building_sector(soldier.element.sector()),
                        eye_z: (!dead).then(|| {
                            soldier.element.position().z
                                + crate::stealth::detection_z_for_posture(posture, is_rider)
                        }),
                        detection_point: (!dead).then(|| {
                            crate::stealth::detection_point_world(
                                position_world,
                                posture,
                                soldier.element.direction(),
                                is_rider,
                            )
                        }),
                        ground_z: soldier.element.position().z,
                        direction: soldier.element.direction(),
                        active: soldier.element.active,
                        unconscious: soldier.human.unconscious,
                        passing_door: soldier.actor.active_door_pass.is_some(),
                        obstacle_idx: soldier.element.obstacle_index(),
                        is_pc: false,
                        is_soldier: true,
                        dead,
                        hollow_man: soldier.human.hollow_man,
                        guarded: false,
                        detection_speed_in_forest: 100,
                        detection_speed_in_city: 100,
                        order_type: crate::order::OrderType::WaitingUpright,
                        blipped: soldier.element.blipped,
                    })
                }
                Entity::Civilian(_) => None,
                _ => unreachable!("Entities::humans returned a non-human entity"),
            })
            .collect()
    }

    /// Live `ComputeVisibility(human) > 0` for one NPC viewer and one human
    /// target, issued outside the batched per-frame detection pass.
    ///
    /// Engine code that asks "does this NPC see that human *right now*" must
    /// go through here rather than reading a `Detectable::seen_now` flag: the
    /// flag is a snapshot taken at the viewer's own detection cadence, so
    /// reusing it both answers a stale question and suppresses the LOS ray the
    /// Original issues at the asking site.
    ///
    /// Only the view-radius memo is shared with the batched pass — that memo
    /// is keyed by viewer, surface, and frame in the Original too, so a
    /// same-frame call legitimately reuses it and, like the Original, still
    /// re-runs the cone test and the opaque-reachability ray.
    pub(crate) fn npc_is_detecting_human(
        &mut self,
        assets: &LevelAssets,
        viewer_id: EntityId,
        target_id: EntityId,
        universal_frame: u32,
    ) -> bool {
        let viewer_building_sector = {
            let Some(entity) = self.world.entities.get(viewer_id) else {
                tracing::warn!(
                    viewer = ?viewer_id,
                    target = ?target_id,
                    "Live detection query skipped: viewer entity missing"
                );
                return false;
            };
            self.entity_building_sector(entity.element_data().sector())
        };
        let Some(viewer) = self.world.entities.get(viewer_id).and_then(|entity| {
            SoldierSightContext::from_npc_viewer(viewer_id, entity, viewer_building_sector)
        }) else {
            // Blind, tied, unconscious, dead, or simply not an NPC: no view
            // parameters exist, which is the same answer the Original's
            // eye-status and posture gates produce.
            tracing::trace!(
                viewer = ?viewer_id,
                target = ?target_id,
                "Live detection query: viewer has no active NPC view state"
            );
            return false;
        };

        let Some(target) = self.world.entities.get(target_id) else {
            tracing::warn!(
                viewer = ?viewer_id,
                target = ?target_id,
                "Live detection query skipped: target entity missing"
            );
            return false;
        };
        let Some(target_human) = target.human_data() else {
            tracing::warn!(
                viewer = ?viewer_id,
                target = ?target_id,
                "Live detection query skipped: target is not a human"
            );
            return false;
        };
        let target_element = target.element_data();
        let target_posture = target_element.posture;
        let target_is_rider = matches!(target, Entity::Soldier(soldier) if soldier.soldier.rider);
        let target_position = target_element.position_map();
        let target_direction = target_element.direction();
        let target_los =
            crate::stealth::detection_point_xy(target_position, target_posture, target_direction);
        let target_detection = crate::stealth::detection_point_world(
            target_element.position(),
            target_posture,
            target_direction,
            target_is_rider,
        );
        let target_obstacle_handle = target_element.obstacle_index();
        let target_building_sector = self.entity_building_sector(target_element.sector());
        let target_active = target_element.active;
        let target_is_pc = matches!(target, Entity::Pc(_));
        let target_action_state = target
            .actor_data()
            .map(|actor| actor.action_state)
            .unwrap_or(crate::element::ActionState::Waiting);
        let target_passing_door = target
            .actor_data()
            .is_some_and(|actor| actor.active_door_pass.is_some());
        let target_unconscious = target_human.unconscious;

        let viewer_in_building = self.entity_building_sector(viewer.sector).is_some();
        let target_in_same_building = viewer_in_building
            && self.entity_building_sector(viewer.sector) == target_building_sector;
        let is_night_or_fog = matches!(
            self.world.weather.ambiance,
            crate::engine::types::Ambiance::Night | crate::engine::types::Ambiance::Fog
        );
        let sight_obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        let target_obstacle = target_obstacle_handle.map(|handle| {
            sight_obstacles.get(usize::from(handle)).unwrap_or_else(|| {
                panic!(
                    "Live detection target {} requires missing sight obstacle {}",
                    target_id.index(),
                    u16::from(handle)
                )
            })
        });

        let q = ai_vision::VisibilityQuery {
            viewer_los: viewer.eye,
            viewer_world: viewer.eye_world,
            viewer_direction: viewer.dir,
            view_forward: viewer.view_forward,
            view_radius: viewer.view_radius,
            viewer_eye_status: viewer.eye_status,
            real_half_aperture: viewer.real_half_aperture,
            viewer_in_building,
            target_in_same_building,
            forest_180_degree_view: forest_180_degree_view_enabled(
                self.world.weather.is_forest_level,
                viewer.camp,
            ),
            golden_eye_mode: self.ai.global.golden_eye_mode,
            effective_view_radius: viewer.view_radius as f32,
            target_is_active_and_outside_building: target_active
                && target_building_sector.is_none(),
            target_los,
            target_world: target_detection,
            target_posture,
            target_action_state,
            target_is_pc,
            sight_obstacles,
            fast_grid: &self.world.fast_grid,
            layer: viewer.layer,
            target_unconscious,
            target_passing_door,
        };

        let view_radius_cache = OwnerViewRadiusCache::from_persistent(
            &self.ai.view_radius_cache,
            viewer_id,
            universal_frame,
        );
        let visibility = ai_vision::compute_visibility_with_effective_radius(&q, || {
            view_radius_cache.get_or_compute(target_obstacle_handle, || {
                ai_vision::compute_view_radius(
                    q.viewer_world,
                    viewer.view_radius,
                    viewer.view_forward,
                    viewer.real_half_aperture,
                    is_night_or_fog,
                    &self.world.fast_grid,
                    sight_obstacles,
                    target_obstacle,
                )
            })
        });
        view_radius_cache.commit_to(&mut self.ai.view_radius_cache, viewer_id, universal_frame);

        tracing::trace!(
            viewer = ?viewer_id,
            target = ?target_id,
            visibility,
            "Live detection query"
        );
        visibility > 0.0
    }

    // ── P3c. Per-NPC non-Enemy detection (Body / Object /
    //         Friend / MissedFriend / Beggar) ────────────────────
    //
    // Per-`type` outer arms of `RefreshDetection` for every
    // detectable bucket except `DETECTABLE_ENEMY` (which is handled
    // by the shared civilian/both-camp mixed PC/soldier walk earlier
    // in the tick). Runs after that pass settles so each NPC's
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
        view_radius_cache: &OwnerViewRadiusCache,
    ) {
        use crate::ai::AiState;

        // -- Read NPC view-state in a scoped read borrow --
        let (viewer, viewer_inside_building) = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            // RefreshDetection runs the per-type loop for both camps.
            // Royalist wrappers return zero for every non-Enemy type, but
            // cleanup, outer scan bookkeeping, latches, and suspect decay
            // still execute.
            let building_sector = self.entity_building_sector(entity.element_data().sector());
            let Some(viewer) =
                SoldierSightContext::from_npc_viewer(npc_id, entity, building_sector)
            else {
                return;
            };
            (
                viewer,
                self.entity_data_inside_building(entity.element_data()),
            )
        };
        let eye = viewer.eye;
        let eye_world = viewer.eye_world;
        let dir = viewer.dir;
        let layer = viewer.layer;
        let view_radius = viewer.view_radius;
        let eye_status = viewer.eye_status;
        let current_state = viewer.current_state;
        let view_forward = viewer.view_forward;
        let real_half_aperture = viewer.real_half_aperture;
        let npc_posture = viewer.posture;
        let current_substate = viewer.current_substate;
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
        // Per-NPC frame phase offset.
        let modified_frame = refresh_detection_modified_frame(
            universal_frame,
            self.original_static_creation_order(npc_id),
        );

        const BODY_DETECTION_FACTOR: f32 = 3.0;

        // Reusable view-speed for `sharpness = view_speed * visibility`.
        let view_speed = if npc_posture == Posture::LeaningOut {
            ai_vision::LOOK_DOWN_BASE_VIEW_SPEED
        } else {
            ai_vision::BASE_VIEW_SPEED
        };

        // Pull the obstacle view + NPC mut borrow for the rest of the
        // function. RefreshDetection is defined by RHElementActorNPC and is
        // therefore shared by soldiers and civilians in the Original.
        let sight_obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        let _ai_global = &mut self.ai.global;
        let Some(npc) = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(Entity::npc_data_mut)
        else {
            return;
        };

        // ── BODY pass ───────────────────────────────────────
        Self::run_human_detectable_pass(
            npc,
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
            // Per-target pre-filter — Body has no extra check. Original
            // compares the full 3D eye/detection points across layers.
            |_t| true,
            ViewContext {
                ground_position: viewer.ground_position,
                viewer_inside_building,
                camp: viewer.camp,
                eye,
                eye_world,
                dir,
                layer,
                view_forward,
                view_radius,
                real_half_aperture,
                viewer_in_building,
                viewer_building_sector,
                is_night_or_fog,
                level: &self.world.fast_grid.level,
                view_radius_cache,
                eye_status,
                view_speed,
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
            npc,
            npc_id,
            ai_vision::DETECTION_FREQUENCY_OBJECT,
            // InstantDetection for OBJECT (Lacklandists) is
            // `!matches!(state, Sleeping|Default)` — Wondering IS
            // instant for Objects.
            !matches!(current_state, AiState::Sleeping | AiState::Default),
            object_targets,
            ViewContext {
                ground_position: viewer.ground_position,
                viewer_inside_building,
                camp: viewer.camp,
                eye,
                eye_world,
                dir,
                layer,
                view_forward,
                view_radius,
                real_half_aperture,
                viewer_in_building,
                viewer_building_sector,
                is_night_or_fog,
                level: &self.world.fast_grid.level,
                view_radius_cache,
                eye_status,
                view_speed,
                modified_frame,
                universal_frame,
                golden_eye,
                sight_obstacles: &sight_obstacles,
                fast_grid: &self.world.fast_grid,
            },
        );

        // ── FRIEND pass ─────────────────────────────────────
        Self::run_human_detectable_pass(
            npc,
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
                ground_position: viewer.ground_position,
                viewer_inside_building,
                camp: viewer.camp,
                eye,
                eye_world,
                dir,
                layer,
                view_forward,
                view_radius,
                real_half_aperture,
                viewer_in_building,
                viewer_building_sector,
                is_night_or_fog,
                level: &self.world.fast_grid.level,
                view_radius_cache,
                eye_status,
                view_speed,
                modified_frame,
                universal_frame,
                golden_eye,
                sight_obstacles: &sight_obstacles,
                fast_grid: &self.world.fast_grid,
            },
        );

        // ── MISSED_FRIEND pass ──────────────────────────────
        Self::run_human_detectable_pass(
            npc,
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
            |t| !missed_friend_or_beggar_target_blocked(t.dead, t.unconscious),
            ViewContext {
                ground_position: viewer.ground_position,
                viewer_inside_building,
                camp: viewer.camp,
                eye,
                eye_world,
                dir,
                layer,
                view_forward,
                view_radius,
                real_half_aperture,
                viewer_in_building,
                viewer_building_sector,
                is_night_or_fog,
                level: &self.world.fast_grid.level,
                view_radius_cache,
                eye_status,
                view_speed,
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
            npc.detectable_lists[beggar_idx].retain(|det| {
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
            npc,
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
            |t| !missed_friend_or_beggar_target_blocked(t.dead, t.unconscious),
            ViewContext {
                ground_position: viewer.ground_position,
                viewer_inside_building,
                camp: viewer.camp,
                eye,
                eye_world,
                dir,
                layer,
                view_forward,
                view_radius,
                real_half_aperture,
                viewer_in_building,
                viewer_building_sector,
                is_night_or_fog,
                level: &self.world.fast_grid.level,
                view_radius_cache,
                eye_status,
                view_speed,
                modified_frame,
                universal_frame,
                golden_eye,
                sight_obstacles: &sight_obstacles,
                fast_grid: &self.world.fast_grid,
            },
        );

        // Original performs this reset after the complete detectable-type
        // loop. A Body/Object suspect retained across a closed cadence must
        // keep its earlier worst type even when Enemy is currently zero.
        finalize_detection_summary(npc);
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
        npc: &mut crate::element::NpcData,
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
        // Original `ComputeVisibility(RHDetectable&)` only applies
        // `bRefreshAlways` to Lacklandist ENEMY entries. Body, Friend,
        // MissedFriend and Beggar always retain their cached visibility until
        // their own modulo cadence opens, even at Yellow/Red alert or while
        // staring/following.
        let gate_open = ctx.modified_frame.is_multiple_of(frequency);

        let mut sum_of_sharpnesses: u16 = 0;
        let mut max_sharpness: u32 = 0;

        // (1) Per-detectable visibility pass.
        for det in npc.detectable_lists[kind_idx].iter_mut() {
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
            if !refresh_detection_scans_target(
                det.last_visibility,
                ctx.viewer_inside_building,
                ctx.ground_position,
                ctx.view_radius,
                target.ground_position,
            ) {
                det.seen_now = false;
                det.last_visibility = 0.0;
                continue;
            }
            tracing::trace!(
                observer = ?npc_id,
                target = ?target_id,
                ?kind,
                frequency,
                modified_frame = ctx.modified_frame,
                gate_open,
                viewer_x = ctx.ground_position.x,
                viewer_y = ctx.ground_position.y,
                "non-Enemy detectable inside RefreshDetection box"
            );
            let visibility: f32 = if non_enemy_visibility_blocked_before_cadence(
                ctx.eye_status,
                ctx.camp,
                extra_gate_blocks_visibility
                    || ctx.viewer_in_building
                    || !target_pre_filter(target),
            ) {
                0.0
            } else if gate_open {
                let target_in_same_building = ctx.viewer_in_building
                    && ctx.viewer_building_sector == target.building_sector
                    && !target.unconscious;
                let target_obstacle_handle = target.obstacle_idx;
                let target_obstacle = target_obstacle_handle.map(|handle| {
                    ctx.sight_obstacles
                        .get(usize::from(handle))
                        .unwrap_or_else(|| {
                            panic!(
                                "{:?} visibility target {} requires missing obstacle {}",
                                kind,
                                target_id.index(),
                                u16::from(handle)
                            )
                        })
                });
                let q = ai_vision::VisibilityQuery {
                    viewer_los: ctx.eye,
                    viewer_world: ctx.eye_world,
                    viewer_direction: ctx.dir,
                    view_forward: ctx.view_forward,
                    view_radius: ctx.view_radius,
                    viewer_eye_status: ctx.eye_status,
                    real_half_aperture: ctx.real_half_aperture,
                    viewer_in_building: ctx.viewer_in_building,
                    target_in_same_building,
                    forest_180_degree_view: false,
                    golden_eye_mode: ctx.golden_eye,
                    effective_view_radius: ctx.view_radius as f32,
                    target_is_active_and_outside_building: target.active
                        && target.building_sector.is_none(),
                    target_los: crate::stealth::detection_point_xy(
                        target.position,
                        target.posture,
                        target.direction,
                    ),
                    target_world: target.detection_point,
                    target_posture: target.posture,
                    target_action_state: target.action_state,
                    target_is_pc: target.is_pc,
                    sight_obstacles: *ctx.sight_obstacles,
                    fast_grid: ctx.fast_grid,
                    layer: ctx.layer,
                    target_unconscious: target.unconscious,
                    target_passing_door: target.passing_door,
                };
                factor
                    * ai_vision::compute_visibility_with_effective_radius(&q, || {
                        ctx.view_radius_cache
                            .get_or_compute(target_obstacle_handle, || {
                                ai_vision::compute_view_radius(
                                    q.viewer_world,
                                    ctx.view_radius,
                                    ctx.view_forward,
                                    ctx.real_half_aperture,
                                    ctx.is_night_or_fog,
                                    ctx.fast_grid,
                                    *ctx.sight_obstacles,
                                    target_obstacle,
                                )
                            })
                    })
            } else {
                det.last_visibility
            };

            let sharpness = detection_sharpness(ctx.view_speed, visibility);
            let is_visible = sharpness > 0;
            max_sharpness = max_sharpness.max(u32::from(sharpness));

            if !det.seen_last_frame {
                sum_of_sharpnesses = accumulate_detection_sharpness(sum_of_sharpnesses, sharpness);
            }

            det.seen_now = is_visible;
            // The outer RefreshDetection loop writes the wrapper result on
            // every scanned entry. Closed cadence reuses the same value;
            // blind/type/camp rejections must overwrite a stale sample with 0.
            det.last_visibility = visibility;
        }

        // `muwMaximalVisibility` spans the complete outer detectable-type
        // loop, not only Enemy entries. Preserve the Enemy maximum installed
        // by the preceding pass and fold this type's post-cache sharpness in.
        if let Some(ai) = npc.ai_brain.base_mut() {
            ai.max_visibility = ai.max_visibility.max(max_sharpness);
        }

        // (2) Snapshot the suspect accumulator. Original
        // HandlePredetection reads this value before the current scan is
        // added below.
        let suspects_before = npc.detection_suspects[kind_idx];

        // (3) HandlePredetection shadow events for PC-typed targets.
        // Body is the only kind that fires; the helper is gated on
        // `fire_shadow_for_pc_targets` so the Friend / MissedFriend
        // / Beggar pre-empt and the Object skip fall out naturally.
        // Per-detectable rising edge of
        //   shadow_is_seen = (sharpness > 0)
        //                && (suspects_before_scan[type]
        //                    >= SHADOW_DETECTION_THRESHOLD)
        //
        // Skip PCs already in custody (guarded) — once a soldier is
        // guarding a hero, no further shadow events fire for that hero, and
        // HandlePredetection leaves its shadow latch unchanged.
        let mut shadow_dispatches: Vec<crate::ai::Position> = Vec::new();
        if fire_shadow_for_pc_targets {
            for det in npc.detectable_lists[kind_idx].iter_mut() {
                // Only PCs are seen as shadows.
                let Some(target_id) = det.element else {
                    continue;
                };
                let Some(target) = targets.get(&target_id) else {
                    continue;
                };
                let shadow_seen_before = det.shadow_seen_last_frame;
                let queued = update_predetection_shadow_latch(
                    det.seen_now,
                    suspects_before,
                    target.is_pc,
                    target.guarded,
                    &mut det.shadow_seen_last_frame,
                );
                tracing::trace!(
                    target: "shadow_predetection",
                    frame = ctx.universal_frame,
                    observer = ?npc_id,
                    observer_index = npc_id.index(),
                    detectable_type = ?kind,
                    detectable_target = ?target_id,
                    detectable_target_index = target_id.index(),
                    sharpness = u32::from(detection_sharpness(
                        ctx.view_speed,
                        det.last_visibility,
                    )),
                    suspects_before,
                    is_pc = target.is_pc,
                    guarded = target.guarded,
                    seen_now = det.seen_now,
                    seen_last_frame = det.seen_last_frame,
                    shadow_seen_now = det.shadow_seen_now,
                    shadow_seen_before,
                    shadow_seen_after = det.shadow_seen_last_frame,
                    last_visibility = det.last_visibility,
                    queued,
                    "evaluated non-Enemy shadow predetection edge"
                );
                if queued {
                    shadow_dispatches.push(crate::ai::Position {
                        x: target.position.x,
                        y: target.position.y,
                        sector: target.sector,
                        level: target.layer,
                    });
                }
            }
        }

        // (4) Accumulate and determine whether the full detection commits.
        let suspects_after = suspects_before.wrapping_add(sum_of_sharpnesses);
        npc.detection_suspects[kind_idx] = suspects_after;
        let commit_threshold = suspects_after >= ai_vision::DETECTION_SUSPECT_THRESHOLD as u16
            || (instant_detection && sum_of_sharpnesses > 0);

        // worst_detected_type bookkeeping — only on visibility
        // frames where new sharpness was added.
        if sum_of_sharpnesses > 0 && (npc.worst_detected_type as u8) > (kind as u8) {
            npc.worst_detected_type = kind;
        }

        // (5) Rising-edge dispatch + drop-on-commit.  When the threshold
        // or instant-detection commits, drop every detectable that
        // crossed the rising edge this frame and queue its event.
        let mut rising_dispatches: Vec<EntityId> = Vec::new();
        if commit_threshold {
            npc.detection_suspects[kind_idx] = 0;
            npc.detectable_lists[kind_idx].retain_mut(|det| {
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

        // (6) Suspect cooldown when this scan added no fresh sharpness.
        npc.detection_suspects[kind_idx] = cool_detection_suspect(
            sum_of_sharpnesses,
            npc.detection_suspects[kind_idx],
            ctx.universal_frame,
        );

        // (7) maximal_detection_suspect contribution
        // (`type < FRIEND` only).
        if contribute_to_maximal && npc.maximal_detection_suspect < npc.detection_suspects[kind_idx]
        {
            npc.maximal_detection_suspect = npc.detection_suspects[kind_idx];
        }

        // (8) Drain the queued stimuli onto pending_stimuli.
        if (!rising_dispatches.is_empty() || !shadow_dispatches.is_empty())
            && let Some(ai) = npc.ai_brain.base_mut()
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
            ai.outbox
                .detection
                .stimuli
                .extend(queued_human_detection_stimuli(
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
        npc: &mut crate::element::NpcData,
        npc_id: EntityId,
        frequency: u32,
        instant_detection: bool,
        targets: &std::collections::HashMap<EntityId, ObjectTarget>,
        ctx: ViewContext<'_>,
    ) {
        let obj_idx = DetectableType::Object as usize;
        // Like the human non-enemy buckets above, OBJECT deliberately ignores
        // the enemy-only `bRefreshAlways` shortcut in the Original.
        let gate_open = ctx.modified_frame.is_multiple_of(frequency);

        // CleanUpDetectables for OBJECT: drop entries whose target
        // is no longer active.  Run before the visibility loop so
        // dead entries don't waste a tick of accumulator decay.
        npc.detectable_lists[obj_idx].retain(|det| {
            let Some(target_id) = det.element else {
                return false;
            };
            targets.get(&target_id).map(|o| o.active).unwrap_or(false)
        });

        let mut sum_of_sharpnesses: u16 = 0;
        let mut max_sharpness: u32 = 0;

        for det in npc.detectable_lists[obj_idx].iter_mut() {
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
            if !refresh_detection_scans_target(
                det.last_visibility,
                ctx.viewer_inside_building,
                ctx.ground_position,
                ctx.view_radius,
                object.ground_position,
            ) {
                det.seen_now = false;
                det.last_visibility = 0.0;
                continue;
            }
            let visibility: f32 = if non_enemy_visibility_blocked_before_cadence(
                ctx.eye_status,
                ctx.camp,
                ctx.viewer_in_building,
            ) {
                0.0
            } else if gate_open {
                let q = ai_vision::ObjectVisibilityQuery {
                    viewer_los: ctx.eye,
                    viewer_world: ctx.eye_world,
                    viewer_direction: ctx.dir,
                    view_forward: ctx.view_forward,
                    view_radius: ctx.view_radius,
                    viewer_eye_status: ctx.eye_status,
                    real_half_aperture: ctx.real_half_aperture,
                    viewer_in_building: ctx.viewer_in_building,
                    object_belongs_to_beggar: object.belongs_to_beggar,
                    target_los: object.position,
                    target_world: object.world_position,
                    sight_obstacles: *ctx.sight_obstacles,
                    fast_grid: ctx.fast_grid,
                    layer: ctx.layer,
                };
                frequency as f32 * ai_vision::compute_object_visibility(&q)
            } else {
                det.last_visibility
            };

            let sharpness = detection_sharpness(ctx.view_speed, visibility);
            let is_visible = sharpness > 0;
            max_sharpness = max_sharpness.max(u32::from(sharpness));
            if !det.seen_last_frame {
                sum_of_sharpnesses = accumulate_detection_sharpness(sum_of_sharpnesses, sharpness);
            }
            det.seen_now = is_visible;
            // Match the unconditional outer-loop SetLastVisibility write.
            det.last_visibility = visibility;
        }

        if let Some(ai) = npc.ai_brain.base_mut() {
            ai.max_visibility = ai.max_visibility.max(max_sharpness);
        }

        let suspects_after = npc.detection_suspects[obj_idx].wrapping_add(sum_of_sharpnesses);
        npc.detection_suspects[obj_idx] = suspects_after;
        let commit_threshold = suspects_after >= ai_vision::DETECTION_SUSPECT_THRESHOLD as u16
            || (instant_detection && sum_of_sharpnesses > 0);

        if sum_of_sharpnesses > 0
            && (npc.worst_detected_type as u8) > (DetectableType::Object as u8)
        {
            npc.worst_detected_type = DetectableType::Object;
        }

        let mut rising_dispatches: Vec<EntityId> = Vec::new();
        if commit_threshold {
            npc.detection_suspects[obj_idx] = 0;
            npc.detectable_lists[obj_idx].retain_mut(|det| {
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
            && npc.detection_suspects[obj_idx] > 0
            && ctx
                .universal_frame
                .is_multiple_of(ai_vision::UNSUSPECT_FREQUENCY)
        {
            npc.detection_suspects[obj_idx] = npc.detection_suspects[obj_idx].saturating_sub(1);
        }

        if npc.maximal_detection_suspect < npc.detection_suspects[obj_idx] {
            npc.maximal_detection_suspect = npc.detection_suspects[obj_idx];
        }

        if !rising_dispatches.is_empty()
            && let Some(ai) = npc.ai_brain.base_mut()
        {
            for target_id in rising_dispatches {
                let mut stimulus =
                    crate::ai::Stimulus::new(crate::ai::StimulusType::EventSeesObject);
                stimulus.info = crate::ai::StimulusInfo::Object(target_id.index());
                ai.outbox.detection.stimuli.push(stimulus);
                tracing::trace!(
                    npc = ?npc_id,
                    object = ?target_id,
                    "EventSeesObject (rising edge)"
                );
            }
        }
    }
}

#[derive(Default)]
struct OwnerViewRadiusCache {
    values: std::cell::RefCell<
        std::collections::HashMap<Option<crate::position_interface::ObstacleHandle>, f32>,
    >,
}

impl OwnerViewRadiusCache {
    fn from_persistent(
        persistent: &crate::ai_vision::ViewRadiusCache,
        viewer: EntityId,
        frame: u32,
    ) -> Self {
        let cache = Self::default();
        if let Some(radius) = persistent.get(None, viewer, frame) {
            cache.values.borrow_mut().insert(None, radius);
        }
        for index in 0..persistent.obstacles.len() {
            let Some(handle) = u16::try_from(index)
                .ok()
                .and_then(crate::position_interface::ObstacleHandle::new)
            else {
                continue;
            };
            if let Some(radius) = persistent.get(Some(handle), viewer, frame) {
                cache.values.borrow_mut().insert(Some(handle), radius);
            }
        }
        cache
    }

    fn commit_to(
        &self,
        persistent: &mut crate::ai_vision::ViewRadiusCache,
        viewer: EntityId,
        frame: u32,
    ) {
        for (&surface, &radius) in self.values.borrow().iter() {
            persistent.set(surface, viewer, frame, radius);
        }
    }

    fn get_or_compute(
        &self,
        obstacle: Option<crate::position_interface::ObstacleHandle>,
        compute: impl FnOnce() -> f32,
    ) -> f32 {
        if let Some(radius) = self.values.borrow().get(&obstacle).copied() {
            return radius;
        }
        let radius = compute();
        // Original uses zero as the cache-miss sentinel: a zero result from
        // ComputeViewRadius is recomputed on the next eligible target.
        if radius != 0.0 {
            self.values.borrow_mut().insert(obstacle, radius);
        }
        radius
    }
}

/// Read-only NPC view-state bundled for one tick of the per-type
/// detection passes (Body / Friend / MissedFriend / Beggar / Object).
/// Avoids passing 18+ args to each helper.  All fields are derived
/// from the soldier's npc/element state at the start of the per-NPC
/// pass; nothing here mutates.
struct ViewContext<'a> {
    ground_position: GroundPoint,
    /// Original `IsInsideBuilding`: building sector or active door transit.
    /// Used only by RefreshDetection's outer scan-entry alternative.
    viewer_inside_building: bool,
    camp: Camp,
    eye: MapPoint,
    eye_world: crate::coordinates::WorldPoint3D,
    dir: i16,
    layer: u16,
    view_forward: (f32, f32),
    view_radius: u16,
    real_half_aperture: f32,
    viewer_in_building: bool,
    viewer_building_sector: Option<crate::position_interface::SectorHandle>,
    is_night_or_fog: bool,
    level: &'a crate::fast_find_grid::LevelGrid,
    view_radius_cache: &'a OwnerViewRadiusCache,
    eye_status: crate::element::EyeStatus,
    view_speed: u16,
    modified_frame: u32,
    universal_frame: u32,
    golden_eye: bool,
    sight_obstacles: &'a crate::sight_obstacle::ObstacleList<'a>,
    fast_grid: &'a crate::fast_find_grid::FastFindGrid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_sharpness_accumulation_wraps_as_uword() {
        assert_eq!(accumulate_detection_sharpness(u16::MAX - 5, 10), 4);
    }

    #[test]
    fn already_seen_enemy_zero_fresh_sharpness_allows_suspect_cooldown() {
        // Savegame_018 replay-011 frame 8660: Soldier 65 still sees its
        // already-latched PC, but that target adds no fresh sharpness.
        assert_eq!(cool_detection_suspect(0, 40, 8660), 39);
        assert_eq!(cool_detection_suspect(1, 40, 8660), 40);
        assert_eq!(cool_detection_suspect(0, 40, 8661), 40);
    }
    use crate::ai::{Position, Substate};
    use crate::element::Posture;

    #[test]
    fn closed_cadence_beggar_disguise_turns_reused_visibility_into_cached_zero() {
        let mut got_beggar_trick = false;
        let visibility = apply_enemy_beggar_disguise(
            Camp::Lacklandists,
            true,
            &mut got_beggar_trick,
            crate::order::OrderType::SimulatingBeggar,
            16.0,
        );
        assert_eq!(visibility, 0.0);
        assert!(!got_beggar_trick);

        let visibility = apply_enemy_beggar_disguise(
            Camp::Lacklandists,
            true,
            &mut got_beggar_trick,
            crate::order::OrderType::TransitionWaitingUprightSimulatingBeggar,
            16.0,
        );
        assert_eq!(visibility, 16.0);
        assert!(got_beggar_trick);
    }

    #[test]
    fn missed_friend_and_beggar_reject_dead_or_unconscious_before_cadence() {
        assert!(!missed_friend_or_beggar_target_blocked(false, false));
        assert!(missed_friend_or_beggar_target_blocked(true, false));
        assert!(missed_friend_or_beggar_target_blocked(false, true));
        assert!(missed_friend_or_beggar_target_blocked(true, true));
    }

    #[test]
    fn blind_type_gate_and_royalist_block_non_enemy_visibility_before_cadence() {
        assert!(non_enemy_visibility_blocked_before_cadence(
            crate::element::EyeStatus::Closed,
            Camp::Lacklandists,
            false,
        ));
        assert!(non_enemy_visibility_blocked_before_cadence(
            crate::element::EyeStatus::LookForward,
            Camp::Lacklandists,
            true,
        ));
        assert!(non_enemy_visibility_blocked_before_cadence(
            crate::element::EyeStatus::LookForward,
            Camp::Royalists,
            false,
        ));
        assert!(!non_enemy_visibility_blocked_before_cadence(
            crate::element::EyeStatus::LookForward,
            Camp::Lacklandists,
            false,
        ));
    }

    #[test]
    fn refresh_detection_outer_box_matches_original_entry_alternatives() {
        let viewer = GroundPoint::new(100.0, 200.0);
        let radius = 80_u16;
        let radius_y = radius as f32 * crate::position_interface::ASPECT_RATIO;
        let far = GroundPoint::new(1000.0, 1000.0);

        assert!(refresh_detection_scans_target(
            0.25, false, viewer, radius, far
        ));
        assert!(refresh_detection_scans_target(
            0.0, true, viewer, radius, far
        ));
        assert!(refresh_detection_scans_target(
            0.0,
            false,
            viewer,
            radius,
            GroundPoint::new(viewer.x + radius as f32, viewer.y + radius_y),
        ));
        assert!(!refresh_detection_scans_target(
            0.0,
            false,
            viewer,
            radius,
            GroundPoint::new(viewer.x + radius as f32 + 0.25, viewer.y),
        ));
        assert!(!refresh_detection_scans_target(
            f32::NAN,
            false,
            viewer,
            radius,
            GroundPoint::new(viewer.x, viewer.y + radius_y + 0.25),
        ));
    }

    #[test]
    fn refresh_detection_outer_box_uses_ground_not_projected_map_y() {
        // Continue/replay-012: Civilian 64 and PC 343 are only 157.57 units
        // apart in Original GetPositionGround Y. Their projected map Y differs
        // by 471.49 because the PC stands roughly 314 units lower. A map-space
        // broad phase incorrectly suppresses the visibility query entirely.
        let viewer_ground = GroundPoint::new(491.0, 1135.000_98);
        let target_ground = GroundPoint::new(668.028_6, 1292.568_2);
        assert!(refresh_detection_scans_target(
            0.0,
            false,
            viewer_ground,
            400,
            target_ground,
        ));

        let viewer_map = MapPoint::new(491.0, 715.0);
        let target_map = MapPoint::new(668.028_6, 1186.492_8);
        let map_radius_y = 400.0 * crate::position_interface::ASPECT_RATIO;
        assert!((target_map.y - viewer_map.y).abs() > map_radius_y);
    }

    #[test]
    fn owner_view_radius_cache_computes_once_per_ground_or_obstacle_key() {
        let cache = OwnerViewRadiusCache::default();
        let calls = std::cell::Cell::new(0_u32);
        let compute = || {
            calls.set(calls.get() + 1);
            321.0
        };

        assert_eq!(cache.get_or_compute(None, compute), 321.0);
        assert_eq!(cache.get_or_compute(None, compute), 321.0);
        assert_eq!(calls.get(), 1);

        let obstacle = crate::position_interface::ObstacleHandle::new(1)
            .expect("test obstacle handle should be representable");
        assert_eq!(cache.get_or_compute(Some(obstacle), compute), 321.0);
        assert_eq!(cache.get_or_compute(Some(obstacle), compute), 321.0);
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn owner_view_radius_cache_does_not_reuse_original_zero_sentinel() {
        let cache = OwnerViewRadiusCache::default();
        let calls = std::cell::Cell::new(0_u32);
        let compute_zero = || {
            calls.set(calls.get() + 1);
            0.0
        };

        assert_eq!(cache.get_or_compute(None, compute_zero), 0.0);
        assert_eq!(cache.get_or_compute(None, compute_zero), 0.0);
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn listen_distance_uses_world_y_before_isometric_stretch() {
        use crate::coordinates::WorldPoint3D;
        use crate::position_interface::INVERSE_ASPECT_RATIO;

        const LIMIT_SQUARED: f32 = 750.0 * 750.0;

        // Derby frame 988: the listener is on the ground and Soldier 71 is
        // elevated. Original includes elevation in world Y before stretching
        // it, leaving the soldier just inside Listen range. The old projected
        // map calculation incorrectly leaves it outside.
        let listener = WorldPoint3D::new(1061.0, 2717.0, 0.0);
        let soldier = WorldPoint3D::new(1079.0, 2300.001_007, 150.001_007);
        assert!(listen_distance_squared(listener, soldier) < LIMIT_SQUARED);
        let projected_dy = (2150.0 - 2717.0) * INVERSE_ASPECT_RATIO;
        let old_projected_square =
            18.0_f32.powi(2) + projected_dy.powi(2) + 150.001_007_f32.powi(2);
        assert!(old_projected_square >= LIMIT_SQUARED);

        // Leicester frame 402 exercises the opposite sign: map-only Y places
        // Civilian 74 inside the sphere, but positive elevation increases
        // world Y separation and Original correctly keeps it blipped.
        let listener = WorldPoint3D::new(1130.0, 248.0, 0.0);
        let civilian = WorldPoint3D::new(738.0, 740.001_007, 140.001_007);
        assert!(listen_distance_squared(listener, civilian) >= LIMIT_SQUARED);
        let projected_dy = (600.0 - 248.0) * INVERSE_ASPECT_RATIO;
        let old_projected_square =
            392.0_f32.powi(2) + projected_dy.powi(2) + 140.001_007_f32.powi(2);
        assert!(old_projected_square < LIMIT_SQUARED);
    }

    #[test]
    fn visibility_eye_projection_keeps_eye_height_out_of_map_y() {
        let eye = crate::coordinates::WorldPoint3D::new(100.0, 260.0, 75.0);
        let projected = visibility_eye_xy(eye, 30.0);

        assert_eq!(projected, MapPoint::new(100.0, 230.0));
        assert_ne!(
            projected,
            eye.to_map(),
            "eye height must not be projected into the LOS point"
        );
    }

    #[test]
    fn elevated_blip_range_uses_world_eye_y_not_projected_map_y() {
        // Soldier 58 and Robin at Original parity frame 23.  The elevated
        // soldier is just inside the 1.5 * 400 world-eye radius.  Using map Y
        // here would incorrectly count the 218-unit elevation difference in
        // both Y and Z and leave the soldier blipped until frame 71.
        let pc_eye = crate::coordinates::WorldPoint3D::new(1937.0, 1604.000_976_562_5, 265.001_007);
        let blip_eye = crate::coordinates::WorldPoint3D::new(2494.211_7, 1623.488_9, 483.001_007);

        assert!(sees_blip_in_range(
            pc_eye,
            blip_eye,
            400.0,
            BLIP_SUPER_DETECTION,
        ));

        let pc_projected = MapPoint::from_world_xyz(pc_eye.x, pc_eye.y, 220.001_007);
        let blip_projected = MapPoint::from_world_xyz(blip_eye.x, blip_eye.y, 438.001_007);
        let incorrectly_projected_eye =
            crate::coordinates::WorldPoint3D::new(blip_projected.x, blip_projected.y, blip_eye.z);
        let incorrectly_projected_pc =
            crate::coordinates::WorldPoint3D::new(pc_projected.x, pc_projected.y, pc_eye.z);
        assert!(!sees_blip_in_range(
            incorrectly_projected_pc,
            incorrectly_projected_eye,
            400.0,
            BLIP_SUPER_DETECTION,
        ));
    }

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
                MapPoint::new(150.0, 170.0),
                posture
            ));
        }

        assert!(!enemy_is_in_react_immediately_zone(
            origin,
            MapPoint::new(150.1, 200.0),
            Posture::Upright
        ));
        assert!(!enemy_is_in_react_immediately_zone(
            origin,
            MapPoint::new(100.0, 230.1),
            Posture::Upright
        ));
        assert!(!enemy_is_in_react_immediately_zone(
            origin,
            MapPoint::new(100.0, 200.0),
            Posture::Spy
        ));
    }

    #[test]
    fn enemy_near_sender_uses_literal_target_map_position_during_door_pass() {
        // schema-14 linux2/Profile_002/Savegame_034 replay-011, frame 35224:
        // Soldier 112 has reached the inside of door 9 while PC 170 is still
        // crossing it. AI `Position(PC 170)` forecasts the far-side door
        // point into the immediate-reaction box, but the Original explicitly
        // reads `GetPositionMap()` and keeps turning until the body itself is
        // within 50x30.
        let owner = MapPoint::new(635.0, 1414.0);
        let forecast_target = MapPoint::new(588.0, 1422.0);
        let literal_target = MapPoint::new(569.884_03, 1423.860_4);

        assert!(enemy_is_in_react_immediately_zone(
            owner,
            forecast_target,
            Posture::Upright
        ));
        assert!(!enemy_is_in_react_immediately_zone(
            owner,
            literal_target,
            Posture::Upright
        ));
    }

    #[test]
    fn enemy_near_sender_only_scans_list_them_and_preserves_order() {
        let origin = MapPoint::new(100.0, 200.0);
        let nearby = |x| MapPoint::new(x, 200.0);
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

    #[test]
    fn predetection_shadow_uses_suspects_from_before_current_scan() {
        let mut shadow_seen_last_frame = false;

        assert!(!update_predetection_shadow_latch(
            true,
            0,
            true,
            false,
            &mut shadow_seen_last_frame,
        ));
        assert!(!shadow_seen_last_frame);

        assert!(update_predetection_shadow_latch(
            true,
            ai_vision::SHADOW_DETECTION_THRESHOLD as u16 + 2,
            true,
            false,
            &mut shadow_seen_last_frame,
        ));
        assert!(shadow_seen_last_frame);
    }

    #[test]
    fn predetection_shadow_early_returns_preserve_the_latch() {
        for (is_pc, guarded) in [(false, false), (true, true)] {
            let mut shadow_seen_last_frame = true;
            assert!(!update_predetection_shadow_latch(
                false,
                ai_vision::SHADOW_DETECTION_THRESHOLD as u16,
                is_pc,
                guarded,
                &mut shadow_seen_last_frame,
            ));
            assert!(shadow_seen_last_frame);
        }
    }

    #[test]
    fn visibility_refresh_gate_uses_view_alert_channel() {
        use crate::ai::AlertLevel;
        use crate::element::EyeStatus;

        assert!(!lacklandist_visibility_refresh_always(
            EyeStatus::LookForward,
            AlertLevel::Green,
        ));
        assert!(lacklandist_visibility_refresh_always(
            EyeStatus::LookForward,
            AlertLevel::Yellow,
        ));
        assert!(lacklandist_visibility_refresh_always(
            EyeStatus::Stare,
            AlertLevel::Green,
        ));
    }

    #[test]
    fn forest_180_degree_view_depends_only_on_level_and_royalist_camp() {
        assert!(forest_180_degree_view_enabled(true, Camp::Royalists));
        assert!(!forest_180_degree_view_enabled(false, Camp::Royalists));
        assert!(!forest_180_degree_view_enabled(true, Camp::Lacklandists));
    }

    #[test]
    fn persistent_body_or_object_suspect_preserves_worst_detected_type() {
        for kind in [DetectableType::Body, DetectableType::Object] {
            let mut npc = crate::element::NpcData::default();
            npc.detection_suspects[kind as usize] = 23;
            npc.maximal_detection_suspect = 23;
            npc.worst_detected_type = kind;

            // No fresh sharpness is required: the per-type fold retains the
            // existing suspect before the complete-loop finalizer runs.
            finalize_detection_summary(&mut npc);
            assert_eq!(npc.worst_detected_type, kind);

            npc.detection_suspects[kind as usize] = 0;
            npc.maximal_detection_suspect = 0;
            finalize_detection_summary(&mut npc);
            assert_eq!(npc.worst_detected_type, DetectableType::None);
        }
    }
}
