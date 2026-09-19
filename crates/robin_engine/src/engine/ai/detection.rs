//! Owner-local visibility work: NPC blip detection and enemy → PC / royalist
//! → enemy detection-refresh dispatch at the matching NPC creation boundary.
//! PC Listen performs its captured-length mixed reveal/Heard scan in the
//! selected PC owner slot; object discovery remains with its object owner.

/// Record-only snapshot taken right after a Listen `ActivatedByListenable`
/// callback returned.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeardCallbackObservation {
    pub target: EntityId,
    /// Whether the target's LISTEN filter was already cleared.
    pub listen_cleared: bool,
    /// Every entity still blipped at that moment, in legacy slot order.
    pub blipped: Vec<EntityId>,
}

#[cfg(test)]
thread_local! {
    static HEARD_CALLBACK_PROBE: crate::engine::test_support::Probe<HeardCallbackObservation> =
        const { crate::engine::test_support::Probe::new() };
}

/// Run `f` and return every Listen Heard callback it performed.
#[cfg(test)]
pub(crate) fn capture_heard_callbacks<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<HeardCallbackObservation>) {
    HEARD_CALLBACK_PROBE.with(|probe| probe.capture(f))
}

#[cfg(test)]
fn observe_heard_callback(engine: &EngineInner, target_id: EntityId) {
    let Some(Entity::Target(target)) = engine.entities().get(target_id) else {
        panic!("Heard callback target {target_id:?} is no longer a target");
    };
    let listen_cleared = !target
        .target
        .action_filter
        .contains(crate::element::TargetFilter::LISTEN);
    let blipped = (0..engine.entities().len())
        .filter_map(|slot| engine.entities().id_at_legacy_slot(slot as u32))
        .filter(|&id| {
            engine
                .entities()
                .get(id)
                .is_some_and(|entity| entity.element_data().blipped)
        })
        .collect();
    HEARD_CALLBACK_PROBE.with(|probe| {
        probe.record(HeardCallbackObservation {
            target: target_id,
            listen_cleared,
            blipped,
        })
    });
}

#[cfg(not(test))]
#[inline(always)]
fn observe_heard_callback(_engine: &EngineInner, _target_id: EntityId) {}

use super::*;
use crate::ai_vision;
use crate::coordinates::{GroundPoint, MapPoint};
use crate::element::{Camp, Detectable, DetectableType, Entity, EntityId, Posture};
use crate::engine::TickCtx;

const DETECTION_FREQUENCY_BLIP: u32 = 16;
const BLIP_SUPER_DETECTION: f32 = 1.5;
const BLIP_ON_SHOULDERS_FACTOR: f32 = 1.3;
const BLIP_CONE_APERTURE_FACTOR: f32 = 1.0;

use crate::engine::diagnostics::ParityGate;
use std::sync::OnceLock;

/// `[frame, creation order]`, both required.
fn hearing_gate_debug_gate() -> &'static ParityGate<2> {
    static GATE: OnceLock<ParityGate<2>> = OnceLock::new();
    GATE.get_or_init(|| {
        ParityGate::from_env_required(
            "PARITY_DEBUG_HEARING_GATE",
            [
                "PARITY_DEBUG_HEARING_GATE_FRAME",
                "PARITY_DEBUG_HEARING_GATE_CREATION_ORDER",
            ],
        )
    })
}

/// `[frame, creation order]`, both required.
fn detectable_list_debug_gate() -> &'static ParityGate<2> {
    static GATE: OnceLock<ParityGate<2>> = OnceLock::new();
    GATE.get_or_init(|| {
        ParityGate::from_env_required(
            "PARITY_DEBUG_DETECTABLE_LIST",
            [
                "PARITY_DEBUG_DETECTABLE_LIST_FRAME",
                "PARITY_DEBUG_DETECTABLE_LIST_CREATION_ORDER",
            ],
        )
    })
}

fn debug_detectable_list_bucket(
    stage: &str,
    bucket: usize,
    npc_id: EntityId,
    npc: &crate::element::AiActorData,
    frame: u32,
    creation_order: u32,
) {
    debug_detectable_list_entries(
        stage,
        bucket,
        npc_id,
        &npc.detectable_lists[bucket],
        frame,
        creation_order,
    );
}

impl EngineInner {
    #[inline(never)]
    fn trace_hearing_gate_pre_gate(
        &self,
        universal_frame: u32,
        npc_id: EntityId,
        creation_order: u32,
        current_state: crate::ai::AiState,
        modified_frame: u32,
        detection_frequency_sounds: u32,
    ) {
        let substate = self
            .entities()
            .get(npc_id)
            .and_then(Entity::ai_controller)
            .expect("HEARINGGATE owner lost its AI controller")
            .current_substate;
        eprintln!(
            "HEARINGGATE {{\"engine\":\"rust\",\"stage\":\"pre_gate\",\"frame\":{},\"owner_slot\":{},\"owner_creation_order\":{},\"state\":{},\"substate\":{},\"modified_frame\":{},\"cadence_remainder\":{},\"state_pass\":{},\"cadence_pass\":{}}}",
            universal_frame,
            npc_id.index(),
            creation_order,
            current_state as u32,
            substate as u32,
            modified_frame,
            modified_frame % detection_frequency_sounds,
            !matches!(current_state, crate::ai::AiState::Attacking),
            modified_frame.is_multiple_of(detection_frequency_sounds),
        );
    }
}

#[inline(never)]
fn trace_hearing_gate_target_outside_box(
    frame_and_creation_order: [u32; 2],
    npc_id: EntityId,
    (pc_id, noise, hear_noise_box): (EntityId, crate::ai::Noise, crate::coordinates::MapBBox),
    positions: (MapPoint, crate::coordinates::WorldPoint3D),
    dets: (bool, bool),
) {
    trace_hearing_gate_target(
        frame_and_creation_order,
        npc_id,
        (pc_id, noise, hear_noise_box),
        positions,
        dets,
        None,
    );
}

/// `inside` is `None` for a hear-box rejection, otherwise
/// `([dx, dy_stretched, dz, modified_volume, max_norm, distance], cover_volume, subjective)`.
#[inline(never)]
fn trace_hearing_gate_target(
    [universal_frame, creation_order]: [u32; 2],
    npc_id: EntityId,
    (pc_id, noise, hear_noise_box): (EntityId, crate::ai::Noise, crate::coordinates::MapBBox),
    (position_map, position_world): (MapPoint, crate::coordinates::WorldPoint3D),
    (det_heard, det_seen): (bool, bool),
    inside: Option<([f32; 6], &dyn std::fmt::Display, &dyn std::fmt::Display)>,
) {
    let pc_volume = noise.volume;
    let (bbox_present, bbox_bits) = hear_noise_box
        .0
        .map(|bbox| {
            (
                true,
                [
                    bbox.min().x.to_bits(),
                    bbox.min().y.to_bits(),
                    bbox.max().x.to_bits(),
                    bbox.max().y.to_bits(),
                ],
            )
        })
        .unwrap_or((false, [0; 4]));
    match inside {
        None => eprintln!(
            "HEARINGGATE {{\"engine\":\"rust\",\"stage\":\"target\",\"frame\":{},\"owner_slot\":{},\"owner_creation_order\":{},\"target_slot\":{},\"inside_box\":false,\"listener_map_bits\":[{},{}],\"listener_world_bits\":[{},{},{}],\"bbox_present\":{},\"bbox_bits\":[{},{},{},{}],\"noise_origin_bits\":[{},{}],\"noise_type\":{},\"noise_volume\":{},\"noise_elevation\":{},\"subjective\":-1,\"old_heard\":{},\"old_seen\":{},\"update\":false}}",
            universal_frame,
            npc_id.index(),
            creation_order,
            pc_id.index(),
            position_map.x.to_bits(),
            position_map.y.to_bits(),
            position_world.x.to_bits(),
            position_world.y.to_bits(),
            position_world.z.to_bits(),
            bbox_present,
            bbox_bits[0],
            bbox_bits[1],
            bbox_bits[2],
            bbox_bits[3],
            noise.origin.x.to_bits(),
            noise.origin.y.to_bits(),
            noise.noise_type as u32,
            pc_volume,
            noise.elevation,
            det_heard,
            det_seen,
        ),
        Some((
            [dx_3d, dy_stretched, dz, modified_volume, max_norm, distance],
            cover_volume,
            subjective,
        )) => eprintln!(
            "HEARINGGATE {{\"engine\":\"rust\",\"stage\":\"target\",\"frame\":{},\"owner_slot\":{},\"owner_creation_order\":{},\"target_slot\":{},\"inside_box\":true,\"listener_map_bits\":[{},{}],\"listener_world_bits\":[{},{},{}],\"bbox_present\":{},\"bbox_bits\":[{},{},{},{}],\"noise_origin_bits\":[{},{}],\"noise_type\":{},\"noise_volume\":{},\"noise_elevation\":{},\"dx_bits\":{},\"dy_stretched_bits\":{},\"dz_bits\":{},\"modified_volume_bits\":{},\"max_norm_bits\":{},\"distance_bits\":{},\"cover_volume\":{},\"subjective\":{},\"old_heard\":{},\"old_seen\":{},\"update\":true}}",
            universal_frame,
            npc_id.index(),
            creation_order,
            pc_id.index(),
            position_map.x.to_bits(),
            position_map.y.to_bits(),
            position_world.x.to_bits(),
            position_world.y.to_bits(),
            position_world.z.to_bits(),
            bbox_present,
            bbox_bits[0],
            bbox_bits[1],
            bbox_bits[2],
            bbox_bits[3],
            noise.origin.x.to_bits(),
            noise.origin.y.to_bits(),
            noise.noise_type as u32,
            pc_volume,
            noise.elevation,
            dx_3d.to_bits(),
            dy_stretched.to_bits(),
            dz.to_bits(),
            modified_volume.to_bits(),
            max_norm.to_bits(),
            distance.to_bits(),
            cover_volume,
            subjective,
            det_heard,
            det_seen,
        ),
    }
}

/// `[universal frame, viewer creation order]`.

#[inline(never)]
fn debug_detectable_list_entries(
    stage: &str,
    bucket: usize,
    npc_id: EntityId,
    entries: &[Detectable],
    frame: u32,
    creation_order: u32,
) {
    if !detectable_list_debug_gate().matches([Some(frame), Some(creation_order)]) {
        return;
    }
    eprintln!(
        "DETLIST {{\"engine\":\"rust\",\"stage\":\"{stage}\",\"frame\":{frame},\"owner_slot\":{},\"owner_creation_order\":{creation_order},\"bucket\":{bucket},\"length\":{}}}",
        npc_id.index(),
        entries.len(),
    );
    for (index, detectable) in entries.iter().enumerate() {
        let target_slot = detectable.element.map(EntityId::index).unwrap_or(u32::MAX);
        eprintln!(
            "DETLIST {{\"engine\":\"rust\",\"stage\":\"{stage}_entry\",\"frame\":{frame},\"owner_slot\":{},\"owner_creation_order\":{creation_order},\"bucket\":{bucket},\"index\":{index},\"target_slot\":{target_slot},\"seen_now\":{},\"seen_last\":{},\"heard_last\":{},\"shadow_now\":{},\"shadow_last\":{},\"last_visibility_bits\":{}}}",
            npc_id.index(),
            detectable.seen_now,
            detectable.seen_last_frame,
            detectable.heard_last_frame,
            detectable.shadow_seen_now,
            detectable.shadow_seen_last_frame,
            detectable.last_visibility.to_bits(),
        );
    }
}

fn debug_all_detectable_list_buckets(
    stage: &str,
    npc_id: EntityId,
    npc: &crate::element::AiActorData,
    frame: u32,
    creation_order: u32,
) {
    for bucket in 0..DetectableType::COUNT {
        debug_detectable_list_bucket(stage, bucket, npc_id, npc, frame, creation_order);
    }
}

/// `[owner slot, owner creation order, (target slot, target creation order) × 3]`,
/// all required.
fn detectable_mutation_debug_gate() -> &'static ParityGate<8> {
    static GATE: OnceLock<ParityGate<8>> = OnceLock::new();
    GATE.get_or_init(|| {
        ParityGate::from_env_required(
            "PARITY_DEBUG_DETECTABLE_MUTATION",
            [
                "PARITY_DEBUG_DETECTABLE_MUTATION_OWNER_SLOT",
                "PARITY_DEBUG_DETECTABLE_MUTATION_OWNER_CREATION_ORDER",
                "PARITY_DEBUG_DETECTABLE_MUTATION_TARGET_0_SLOT",
                "PARITY_DEBUG_DETECTABLE_MUTATION_TARGET_0_CREATION_ORDER",
                "PARITY_DEBUG_DETECTABLE_MUTATION_TARGET_1_SLOT",
                "PARITY_DEBUG_DETECTABLE_MUTATION_TARGET_1_CREATION_ORDER",
                "PARITY_DEBUG_DETECTABLE_MUTATION_TARGET_2_SLOT",
                "PARITY_DEBUG_DETECTABLE_MUTATION_TARGET_2_CREATION_ORDER",
            ],
        )
    })
}

/// `(slot, creation order)` of the three selected targets. Enabled gates only.
fn detectable_mutation_debug_targets() -> impl Iterator<Item = (u32, u32)> {
    let gate = detectable_mutation_debug_gate();
    (0..3).map(|target| (gate.required(2 + 2 * target), gate.required(3 + 2 * target)))
}

pub(super) fn detectable_mutation_debug_enabled() -> bool {
    detectable_mutation_debug_gate().enabled()
}

pub(super) fn detectable_mutation_debug_owner_slot_matches(owner_slot: u32) -> bool {
    detectable_mutation_debug_enabled()
        && detectable_mutation_debug_gate().required(0) == owner_slot
}

pub(super) fn detectable_mutation_debug_target_slot_matches(target_slot: u32) -> bool {
    detectable_mutation_debug_enabled()
        && detectable_mutation_debug_targets().any(|(slot, _)| slot == target_slot)
}

pub(super) fn detectable_mutation_debug_owner_matches(
    owner_slot: u32,
    owner_creation_order: u32,
) -> bool {
    detectable_mutation_debug_owner_slot_matches(owner_slot)
        && detectable_mutation_debug_gate().required(1) == owner_creation_order
}

pub(super) fn detectable_mutation_debug_target_matches(
    target_slot: u32,
    target_creation_order: u32,
) -> bool {
    detectable_mutation_debug_enabled()
        && detectable_mutation_debug_targets().any(|(slot, creation_order)| {
            slot == target_slot && creation_order == target_creation_order
        })
}

#[inline(never)]
pub(super) fn debug_detectable_mutation_event(
    stage: &str,
    caller: &str,
    frame: u32,
    owner_slot: u32,
    owner_creation_order: u32,
    bucket: usize,
    target_slot: u32,
    target_creation_order: u32,
    present_before: bool,
    present_after: bool,
    length_before: usize,
    length_after: usize,
) {
    if !detectable_mutation_debug_owner_matches(owner_slot, owner_creation_order)
        || !detectable_mutation_debug_target_matches(target_slot, target_creation_order)
    {
        return;
    }
    eprintln!(
        "DETMUT {{\"engine\":\"rust\",\"stage\":\"{stage}\",\"caller\":\"{caller}\",\"frame\":{frame},\"owner_slot\":{owner_slot},\"owner_creation_order\":{owner_creation_order},\"bucket\":{bucket},\"target_slot\":{target_slot},\"target_creation_order\":{target_creation_order},\"present_before\":{present_before},\"present_after\":{present_after},\"length_before\":{length_before},\"length_after\":{length_after}}}"
    );
}

fn debug_detectable_mutation_snapshot(
    stage: &str,
    caller: &str,
    frame: u32,
    owner_id: EntityId,
    owner_creation_order: u32,
    detectable_lists: &[Vec<Detectable>],
    creation_order_for: impl Fn(EntityId) -> Option<u32>,
) {
    if !detectable_mutation_debug_owner_matches(owner_id.index(), owner_creation_order) {
        return;
    }
    for (target_slot, target_creation_order) in detectable_mutation_debug_targets() {
        let matching = detectable_lists
            .iter()
            .enumerate()
            .find_map(|(bucket, entries)| {
                entries.iter().find_map(|detectable| {
                    let entity_id = detectable.element?;
                    (entity_id.index() == target_slot
                        && creation_order_for(entity_id) == Some(target_creation_order))
                    .then_some((bucket, entries.len()))
                })
            });
        let (bucket, length) = matching.unwrap_or((usize::MAX, 0));
        debug_detectable_mutation_event(
            stage,
            caller,
            frame,
            owner_id.index(),
            owner_creation_order,
            bucket,
            target_slot,
            target_creation_order,
            matching.is_some(),
            matching.is_some(),
            length,
            length,
        );
    }
}

pub(crate) fn debug_detectable_mutation_load_snapshot(
    owner_id: EntityId,
    owner_creation_order: u32,
    detectable_lists: &[Vec<Detectable>],
    creation_order_for: impl Fn(EntityId) -> Option<u32>,
) {
    debug_detectable_mutation_snapshot(
        "deserialize_snapshot",
        "legacy_save_adopt",
        0,
        owner_id,
        owner_creation_order,
        detectable_lists,
        creation_order_for,
    );
}

/// Eye point of a human, in both spaces the visibility code needs: the
/// projected map point used by the cone / spatial LOS tests, and the
/// world-space eye point the 3D opaque-reachability query
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

/// Exact range half of player-character blip visibility.
///
/// The original game subtracts the world-space eye points and only
/// then applies the isometric Y stretch. The same world-space eye points are
/// also passed to the original game's 3D opaque-reachability query.
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

/// Exact distance half of player-character listening.
///
/// The original game subtracts the elements' full world positions and
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

fn difficulty_hearing_factor(
    hostile_soldier: bool,
    difficulty: crate::player_profile::DifficultyLevel,
) -> f32 {
    if hostile_soldier {
        difficulty.rules().hostile_soldier_noise_factor()
    } else {
        1.0
    }
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

pub(super) fn forest_180_degree_view_enabled_with_relationship(
    is_forest_level: bool,
    viewer_player_aligned: bool,
) -> bool {
    is_forest_level && viewer_player_aligned
}

/// Original clears the remembered worst type only after every detectable
/// bucket has contributed its persistent suspect value to the frame maximum.
fn finalize_detection_summary(npc: &mut crate::element::AiActorData) {
    if npc.maximal_detection_suspect == 0 {
        npc.worst_detected_type = DetectableType::None;
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

fn non_enemy_visibility_blocked_with_relationship(
    eye_status: crate::element::EyeStatus,
    viewer_hostile_to_player: bool,
    type_gate_blocked: bool,
) -> bool {
    eye_status.is_blind() || !viewer_hostile_to_player || type_gate_blocked
}

fn apply_enemy_beggar_disguise_with_relationship(
    viewer_hostile_to_player: bool,
    target_is_pc: bool,
    got_beggar_trick: &mut bool,
    order_type: crate::order::OrderType,
    visibility: f32,
) -> f32 {
    if !viewer_hostile_to_player || !target_is_pc || *got_beggar_trick || visibility <= 0.0 {
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

fn achievement_observation_sample(
    is_visible: bool,
    target_is_pc: bool,
    viewer_hostile_to_player: bool,
) -> bool {
    is_visible && target_is_pc && viewer_hostile_to_player
}

/// Original-game NPC predetection shadow-edge update.
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

/// Staggered cadence counter used by NPC detection refresh.
///
/// The original game stores `universal frame + creation order` in an unsigned 16-bit value before
/// applying every blip, sound, and optical modulo gate. The truncation is
/// observable once a mission's universal frame passes 65535.
fn refresh_detection_modified_frame(universal_frame: u32, creation_order: u32) -> u32 {
    universal_frame.wrapping_add(creation_order) as u16 as u32
}

/// The original game's per-type detection cooldown. The sum is fresh
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
    /// Return the exact element creation order assigned by the
    /// Original-compatible construction stream.
    pub(super) fn original_static_creation_order(&self, entity_id: EntityId) -> u32 {
        self.world.original_creation_order(entity_id)
    }

    /// Original-game attacking reaction-time enemy-near test.
    ///
    /// The soldier update calls this before the NPC detection
    /// pass. The gate is evaluated once, then the current enemy list is
    /// walked in order and each eligible nearby enemy is sent through Think.
    pub(crate) fn tick_attacking_reactiontime_enemy_near_for(
        &mut self,
        tcx: TickCtx<'_>,
        npc_id: EntityId,
    ) {
        let frame = self.control.frame_counter;
        let Some(Entity::Soldier(soldier)) = self.entities().get(npc_id) else {
            panic!("nearby-enemy check owner {} disappeared", npc_id.index());
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
        let target_count = enemy_ai.list_them.len();
        for index in 0..target_count {
            let target_handle = *self
                .entities()
                .expect_entity(npc_id, format_args!("nearby-enemy owner"))
                .enemy_ai()
                .expect("nearby-enemy owner lost its enemy brain")
                .list_them
                .get(index)
                .expect("nearby-enemy list shrank during its synchronous scan");
            let target_id = self
                .entity_id_for_index(target_handle)
                .unwrap_or_else(|| panic!("nearby-enemy target {target_handle} disappeared"));
            let target = self
                .entities()
                .expect_entity(target_id, format_args!("nearby-enemy target"));
            assert!(
                target.human_data().is_some(),
                "nearby-enemy target {target_handle} is not human"
            );
            // Keep the owner's entry-time box, but read each target immediately
            // before its callback. Earlier callbacks may move later targets.
            if !enemy_is_in_react_immediately_zone(
                origin,
                target.element_data().position_map(),
                target.element_data().posture(),
            ) {
                continue;
            }
            let stimulus = crate::ai::Stimulus::with_human(
                crate::ai::StimulusType::EventEnemyNear,
                target_handle,
            );
            self.execute_ai_callback(tcx, npc_id, &stimulus);
        }
    }

    /// P2a — non-NPC blip work: drive the Listen ability's one-shot reveal,
    /// and FX-target Heard() callbacks. Ordinary NPC
    /// Blip detection runs inside that NPC's creation-ordered detection refresh.
    pub(super) fn tick_enemy_ai_blip_detection(
        &mut self,
        tcx: TickCtx<'_>,
        pc_id: EntityId,
    ) -> Option<crate::sprite::MotionState> {
        const DISTANCE_LISTEN: f32 = 750.0;
        const TIME_LISTEN_WAIT: u32 = 25;
        // FrozenAll is volatile script state and Original samples it inside
        // sprite action. The Listen countdown still runs through
        // its Execute arm while frozen, but its visual operand must not move.
        let sprite_frozen = self.actors_frozen();
        // ── Listen ability frame tick. ──────────────────────
        // Each frame a PC executes the Listening order:
        //
        //  - Arm `wait_time` to `TIME_LISTEN_WAIT` on order initialization.
        //  - Decrement the countdown. On a nonterminal frame, call `Turn()`
        //    and drive the `LISTENING` sprite while deliberately ignoring its
        //    completion state. On the frame the countdown reaches 0,
        //    fire the one-shot blip reveal + FX-target `Heard()`
        //    callback (below) and return termination to the actor update.
        //
        // The action state stays `Listening` through the
        // countdown — the exit transition in owner-local `tick_ability`
        // will flip it back to `Waiting`.
        let Some(ability) =
            crate::abilities::selected_ability(&self.entities(), &self.seq(), pc_id)
                .filter(|ability| ability.order_type == crate::order::OrderType::Listening)
        else {
            return None;
        };
        let listener_position = {
            let pc = match self.entities_mut().get_mut(pc_id) {
                Some(Entity::Pc(pc)) => pc,
                Some(_) => panic!("Listen owner {pc_id:?} is not a PC"),
                None => panic!("Listen owner {pc_id:?} disappeared"),
            };
            if pc.actor.execute_order_initialising {
                pc.actor.wait_time = TIME_LISTEN_WAIT;
            }
            let reveal = pc.actor.wait_time == 1;
            if pc.actor.wait_time != 0 {
                pc.actor.wait_time -= 1;
            }
            if !reveal {
                // Player-character execution performs the visual action only
                // after the timer's terminal early return. Its sprite result
                // never advances the sequence; the wait timer is authoritative.
                pc.element.sprite.position_iface.turn();
                let direction = pc.element.direction() as u16;
                let order_id = ability.order_id;
                if !sprite_frozen {
                    let _ignored_motion = pc.element.sprite.perform_action(
                        tcx.sim,
                        Some(order_id),
                        crate::order::OrderType::Listening,
                        direction,
                        crate::sprite::FrameProgression::Default,
                        false,
                    );
                }
                // Player-character execution deliberately discards
                // action processing's start/done result for listening and returns
                // an in-progress result on every nonterminal countdown tick.
                return Some(crate::sprite::MotionState::InProgress);
            }
            // Countdown hit 0 — fire the one-shot reveal and
            // advance the phase so owner-local `tick_ability` plays the
            // exit transition next.
            tracing::debug!(
                pc = pc_id.index(),
                "Listen: one-shot reveal fired after TIME_LISTEN_WAIT frames"
            );
            pc.element.position()
        };

        {
            // The original game captures the size once, then resolves each live slot and
            // applies blip reveals and hearing synchronously in that mixed order.
            let captured_len = self.entities().len();
            for slot in 0..captured_len {
                let Some(entity_id) = self.entities().id_at_legacy_slot(slot as u32) else {
                    continue;
                };
                let Some(entity) = self.entities().get(entity_id) else {
                    continue;
                };
                let elem = entity.element_data();
                if listen_distance_squared(listener_position, elem.position())
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
                let fog_reveal = entity.is_human() || reveal;
                if fog_reveal {
                    // Unlike blip reveals, this is temporary intelligence and
                    // never changes the Original's permanent identity flag.
                    self.reveal_entity_from_listen(entity_id);
                }
                if reveal {
                    self.entities_mut()
                        .get_mut(entity_id)
                        .unwrap()
                        .reveal_blip();
                }
                if heard && tcx.sim.config().script_enabled {
                    let target = match self.entities_mut().get_mut(entity_id) {
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
                    let pc_handle = crate::natives::ScriptHandleCodec::actor_handle(pc_id);
                    self.call_script_vm(
                        tcx,
                        ScriptVmKey::Target(target_handle),
                        "ActivatedByListenable",
                        &[pc_handle],
                        crate::natives::ScriptCallFrame::actor(target_handle),
                    )
                    .unwrap_or_else(|error| {
                        panic!("ActivatedByListenable target {target_handle} failed: {error}")
                    });
                    observe_heard_callback(self, entity_id);
                }
            }
        }
        Some(crate::sprite::MotionState::Terminated)
    }

    /// Strict live discovery refresh for one bonus-owned
    /// actor update slot.
    pub(crate) fn refresh_bonus_discovered_for(
        &mut self,
        assets: &LevelAssets,
        bonus_id: EntityId,
    ) {
        let bonus = self.expect_entity(bonus_id, "bonus before discovery refresh");
        let Entity::Bonus(bonus) = bonus else {
            panic!("discovery refresh owner {bonus_id:?} is not Entity::Bonus")
        };
        if !bonus.element.blipped || !bonus.element.active {
            return;
        }
        let bonus_position = bonus.element.position();
        let radius = self.ai.standard_view_polygon_radius as f32;
        let square_standard_view_radius = radius * radius;
        let sight_obstacles = self.sight_obstacles(assets);
        let discovered = self
            .world
            .original_pc_registry()
            .iter()
            .copied()
            .any(|pc_id| {
                let entity = self.entities().expect_entity(
                    pc_id,
                    format_args!("bonus {bonus_id:?} discovery refresh PC registry id"),
                );
                let Entity::Pc(pc) = entity else {
                    panic!(
                        "bonus {bonus_id:?} discovery refresh found non-PC registry id {pc_id:?}"
                    )
                };
                if pc.pc.life_points <= 0 || pc.human.unconscious || !pc.element.active {
                    return false;
                }
                let eyes = entity.compute_eyes_point(None).unwrap_or_else(|| {
                    panic!("bonus {bonus_id:?} could not compute eyes for required PC {pc_id:?}")
                });
                let dx = eyes.x - bonus_position.x;
                let dy =
                    (eyes.y - bonus_position.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
                let dz = eyes.z - bonus_position.z;
                let threshold = if pc.element.posture() == crate::element::Posture::OnShoulders {
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
            self.entities_mut()
                .get_mut(bonus_id)
                .expect("discovered bonus disappeared before clearing its blip")
                .reveal_blip();
        }
    }

    /// NPC-owned detection-refresh blip branch. This must run at the start of
    /// each NPC's creation slot: an earlier NPC's synchronous Think/script
    /// may activate, deactivate, blip, reveal, or move a later NPC before its
    /// own cadence opens.
    fn tick_enemy_ai_npc_blip_detection_for_npc(&mut self, tcx: TickCtx<'_>, npc_id: EntityId) {
        use crate::element::Posture;

        let entity = self.expect_entity(
            npc_id,
            "creation-ordered NPC before its blip detection slot",
        );
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
        // only behind the same detection-refresh entry/cadence gates.
        if matches!(entity, Entity::Soldier(s)
            if self.is_player_aligned_camp(s.soldier.cached_camp))
        {
            self.entities_mut()
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
        let difficulty_factor = crate::player_profile::DifficultyRules::percent_as_f32(
            tcx.sim
                .config()
                .difficulty
                .rules()
                .blip_detection_range_percent,
        );
        let sight_obstacles = self.world.sight_obstacles(tcx.assets);

        let mut detecting_pc = None;
        for &pc_id in self.world.original_pc_registry() {
            let pc_entity =
                self.expect_entity(pc_id, "PC from the live PC list during NPC blip detection");
            let Entity::Pc(pc) = pc_entity else {
                panic!(
                    "non-PC entity {} is present in the live PC list during NPC blip detection",
                    pc_id.index()
                );
            };
            // The original game's detection refresh checks blip visibility for every active,
            // playable, living, conscious PC. Rescue targets can be playable
            // while exposing no command interface and still reveal blips.
            if !pc.element.active
                || !pc.pc.playable
                || pc.pc.life_points <= 0
                || pc.human.unconscious
            {
                continue;
            }
            let (_pc_eye_xy, pc_eye_world) = human_eye_point_for_visibility(pc_entity);
            let super_detection = if pc.element.posture() == Posture::OnShoulders {
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
                detecting_pc = Some((pc_id, pc.element.posture() == Posture::OnShoulders));
                break;
            }
        }

        let Some((pc_id, perched)) = detecting_pc else {
            return;
        };
        self.entities_mut()
            .get_mut(npc_id)
            .expect("blipped NPC disappeared before reveal")
            .reveal_blip();
        if perched {
            self.hero_speaking(
                tcx.assets,
                pc_id,
                crate::engine::melee::HERO_PERCHED_AND_SEE_ENNEMY,
            );
        }
    }

    /// Acoustic portion of one NPC's detection refresh.
    ///
    /// The hearing branch is called per-tick from every NPC's
    /// update, so civilians run it too — which is how they
    /// react to the PC walking by / swordfighting nearby.
    ///
    /// This stays separate from the soldier-only visual helper so civilians
    /// continue to hear PCs. It is nevertheless called from the creation-
    /// ordered per-NPC coordinator: the original game's hearing update invokes
    /// `Think(EVENT_HEAR)` inline, and that state change is visible to the
    /// same NPC's optical `InstantDetection` decision immediately afterward.
    pub(super) fn tick_enemy_ai_acoustic_detection_for_npc(
        &mut self,
        tcx: TickCtx<'_>,
        npc_id: EntityId,
    ) {
        use crate::ai::AiState;

        const DETECTION_FREQUENCY_SOUNDS: u32 = 3;

        let universal_frame = self.control.frame_counter;
        // Read NPC state. The state gate is sampled once before the enemy-list
        // loop, as in the original outer
        // `if (mCurrentState != STATE_ATTACKING)`.
        let (current_state, hearing_factor) = {
            let Some(entity) = self.entities().get(npc_id) else {
                return;
            };
            // Detection refresh's first gate admits inactive NPCs only while
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
            if entity.ai_actor_data().is_none() {
                return;
            }
            if entity.is_dead() || entity.element_data().posture() == Posture::Tied {
                return;
            }
            if entity.is_unconscious() {
                return;
            }
            let Some(npc) = entity.ai_actor_data() else {
                return;
            };
            let hostile_soldier = matches!(entity, Entity::Soldier(_))
                && entity.camp().is_hostile_to(Camp::Royalists);
            let hearing_factor =
                difficulty_hearing_factor(hostile_soldier, tcx.sim.config().difficulty);
            (npc.ai_state(), hearing_factor)
        };
        let hearing_debug_gate = hearing_gate_debug_gate();
        let hearing_debug = hearing_debug_gate.matches([Some(universal_frame), None])
            && hearing_debug_gate
                .matches([None, Some(self.original_static_creation_order(npc_id))]);
        let hearing_debug_creation_order =
            hearing_debug.then(|| self.original_static_creation_order(npc_id));
        let hearing_debug_modified_frame = hearing_debug_creation_order.map(|creation_order| {
            refresh_detection_modified_frame(universal_frame, creation_order)
        });
        if hearing_debug {
            self.trace_hearing_gate_pre_gate(
                universal_frame,
                npc_id,
                hearing_debug_creation_order.expect("HEARINGGATE creation order missing"),
                current_state,
                hearing_debug_modified_frame.expect("HEARINGGATE frame missing"),
                DETECTION_FREQUENCY_SOUNDS,
            );
        }
        // Attacking NPCs are already locked onto their target
        // and don't accumulate new hearing stimuli.
        if matches!(current_state, AiState::Attacking) {
            return;
        }
        let modified_frame = hearing_debug_modified_frame.unwrap_or_else(|| {
            refresh_detection_modified_frame(
                universal_frame,
                self.original_static_creation_order(npc_id),
            )
        });
        if !modified_frame.is_multiple_of(DETECTION_FREQUENCY_SOUNDS) {
            return;
        }

        let enemy_idx = DetectableType::Enemy as usize;
        let target_count = self
            .entities()
            .expect_entity(npc_id, format_args!("hearing owner"))
            .ai_actor_data()
            .expect("hearing owner lost its AI actor data")
            .detectable_lists[enemy_idx]
            .len();
        for index in 0..target_count {
            let listener = self
                .entities()
                .expect_entity(npc_id, format_args!("hearing owner"));
            let target_id = listener
                .ai_actor_data()
                .expect("hearing owner lost its AI actor data")
                .detectable_lists[enemy_idx]
                .get(index)
                .expect("hearing enemy list shrank during its synchronous scan")
                .element
                .expect("hearing enemy list contains a missing target");
            let EntityId::Pc(_) = target_id else { continue };
            let pc_id = target_id;
            let position_map = listener.element_data().position_map();
            let position_world = listener.element_data().position();
            let listener_dead = listener.is_dead();
            // The list length belongs to the outer scan; noise and geometry
            // belong to each individual call after the preceding Think returns.
            let pc = match self
                .entities()
                .expect_entity(pc_id, format_args!("hearing PC"))
            {
                Entity::Pc(pc) => pc,
                _ => unreachable!("typed PC identifier resolved to another entity kind"),
            };
            let noise = pc
                .actor
                .produced_noise
                .expect("hearing PC has no initialized produced-noise record");
            let hear_noise_box = pc.actor.hear_noise_box;
            let is_swordfighting = !pc.human.opponents.is_empty();
            let cover_volume = self
                .feedback
                .sound_sim
                .sources
                .max_noise_covering_volume_for_3d(
                    position_world.x,
                    position_world.y,
                    position_world.z,
                );
            let stimulus = {
                let npc = self
                    .entities_mut()
                    .expect_entity_mut(npc_id, format_args!("hearing latch owner"))
                    .ai_actor_data_mut()
                    .expect("hearing owner lost its AI actor data");
                let pc_volume = noise.volume;
                // Hear-my-noise-box pre-filter. The human stores this box
                // on the PC and does not rebuild it when
                // Produced-noise refresh returns through its
                // inactive/building or quiet-animation arms. It can thus
                // intentionally disagree with the current noise origin
                // and volume; outside the stale box hearing is not
                // called and the edge latch remains untouched.
                // The authored box is sized for a 100% listener. A
                // difficulty-enhanced guard may legitimately hear beyond
                // it, so let the exact 3D max-norm/range checks below make
                // that decision. Reduced sensitivity still uses the box
                // as a cheap outer bound.
                let inside_hear_box =
                    hear_noise_box.contains_point(position_map) || hearing_factor > 1.0;
                if !inside_hear_box {
                    if hearing_debug {
                        trace_hearing_gate_target_outside_box(
                            [
                                universal_frame,
                                hearing_debug_creation_order
                                    .expect("HEARINGGATE creation order missing"),
                            ],
                            npc_id,
                            (pc_id, noise, hear_noise_box),
                            (position_map, position_world),
                            (
                                npc.detectable_lists[enemy_idx][index].heard_last_frame,
                                npc.detectable_lists[enemy_idx][index].seen_last_frame,
                            ),
                        );
                    }
                    None
                } else {
                    // Heard-volume calculation uses the full 3D position. Its noise
                    // origin is `(x, y + elevation, elevation)` and it has
                    // no logical-layer rejection, so nearby cross-layer
                    // sounds remain audible when their actual geometry is.
                    let source_elevation = noise.elevation as f32;
                    let dy_stretched = (position_world.y - noise.origin.y - source_elevation)
                        * crate::position_interface::INVERSE_ASPECT_RATIO;
                    let dx_3d = position_world.x - noise.origin.x;
                    let dz = position_world.z - source_elevation;
                    let modified_volume = pc_volume as f32 * hearing_factor;
                    let max_norm = dx_3d.abs().max(dy_stretched.abs()).max(dz.abs());
                    let distance = (dx_3d * dx_3d + dy_stretched * dy_stretched + dz * dz).sqrt();
                    // Hearing-volume calculation rejects disabled noise,
                    // a coincident source/listener, and sources beyond the
                    // modified-volume max norm. Hearing updates still run
                    // for all of these inside-box cases and clears its
                    // rising-edge latch.
                    let subjective = if pc_volume == 0
                        || distance == 0.0
                        || max_norm > modified_volume
                        || modified_volume - distance <= 0.0
                    {
                        0
                    } else {
                        // Heard-volume calculation checks deafness only after
                        // every semantic/range check and the positive
                        // subjective-volume test. Besides avoiding wasted
                        // work, this preserves the observable cached-frame
                        // mutation when all tracked PCs are inaudible.
                        let deafness = npc.get_deafness(universal_frame, cover_volume);
                        subjective_hear_volume(modified_volume, distance, deafness)
                    };

                    let det = &npc.detectable_lists[enemy_idx][index];
                    let (det_heard, det_seen) = (det.heard_last_frame, det.seen_last_frame);

                    if hearing_debug {
                        trace_hearing_gate_target(
                            [
                                universal_frame,
                                hearing_debug_creation_order
                                    .expect("HEARINGGATE creation order missing"),
                            ],
                            npc_id,
                            (pc_id, noise, hear_noise_box),
                            (position_map, position_world),
                            (det_heard, det_seen),
                            Some((
                                [dx_3d, dy_stretched, dz, modified_volume, max_norm, distance],
                                &cover_volume,
                                &subjective,
                            )),
                        );
                    }

                    let stimulus = if !listener_dead && subjective > 0 && !det_heard && !det_seen {
                        let noise = crate::ai::Noise {
                            origin: noise.origin,
                            noise_type: if is_swordfighting {
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

                    // Hearing updates always refresh this latch when the
                    // hear-box admitted the target, including zero-volume
                    // and beyond-range cases.
                    if !listener_dead {
                        npc.detectable_lists[enemy_idx][index].heard_last_frame = subjective > 0;
                    }
                    stimulus
                }
            };

            let Some(mut stimulus) = stimulus else {
                continue;
            };

            let source_position = self.live_ai_position(pc_id);
            let crate::ai::StimulusInfo::Noise(ref mut heard_noise) = stimulus.info else {
                panic!("periodic hearing edge lost its required noise payload")
            };
            // The decision receives the enemy's current planning position,
            // including door-side and carrier substitution. Volume calculation
            // above uses the produced-noise origin instead.
            heard_noise.origin = crate::ai::NoiseOrigin::from_position(source_position);
            self.execute_ai_callback(tcx, npc_id, &stimulus);
        }
    }

    /// One NPC's contiguous detection refresh. The owner coordinator runs its
    /// inform/view prelude and post-detection tail around this call.
    /// Run synchronous acoustics, select the camp-specific
    /// Enemy visibility path (Lacklandist→PC or Royalist→Lacklandist), then run
    /// the remaining detectable buckets and flush that NPC's complete FIFO
    /// before returning to the owner coordinator. EVENT_VIEW is queued after
    /// the Enemy scan and dispatched only after every detectable bucket has
    /// released the NPC borrow.
    /// Volatile NPC target metadata is rebuilt at each creation slot so a
    /// later NPC observes state changes made by an earlier NPC's Think.
    /// Original:
    /// Detection refresh queues detection stimuli while
    /// scanning lists, then calls `Think` before returning from that NPC's
    /// actor update.
    pub(super) fn tick_enemy_ai_refresh_detection(&mut self, tcx: TickCtx<'_>, npc_id: EntityId) {
        let _detail = super::super::tick::entity_system_detail_guard(
            super::super::tick::EntitySystemDetail::RefreshDetection,
        );
        let universal_frame = self.control.frame_counter;

        self.tick_enemy_ai_npc_blip_detection_for_npc(tcx, npc_id);

        // Sample the two pre-acoustic detection-refresh gates before
        // EVENT_HEAR can synchronously run Think/script and mutate the
        // viewer. Once these gates pass, original control flow always
        // reaches the pre-optical maxima reset.
        let passed_pre_acoustic_gates = self.entities().get(npc_id).is_some_and(|entity| {
            let elem = entity.element_data();
            let entered_refresh = elem.active
                || elem.is_in_door_transit()
                || self.entity_building_sector(elem.sector()).is_some();
            entered_refresh
                && !entity.is_dead()
                && entity.human_data().is_none_or(|human| !human.unconscious)
                && elem.posture() != Posture::Tied
        });
        self.tick_enemy_ai_acoustic_detection_for_npc(tcx, npc_id);

        // Detection refresh clears both maxima after acoustics but
        // before its narrower optical eligibility gate. In particular,
        // an inactive NPC on a door rail reaches this reset and then
        // returns without scanning; an inactive outdoor NPC returned at
        // the entry gate and must retain the old value.
        if passed_pre_acoustic_gates
            && let Some(npc) = self
                .entities_mut()
                .get_mut(npc_id)
                .and_then(Entity::ai_actor_data_mut)
        {
            npc.maximal_detection_suspect = 0;
            if let Some(ai) = npc.ai_brain.base_mut() {
                ai.max_visibility = 0;
            }
        }

        let mut stimuli = Vec::new();
        let detectable_list_debug_creation_order = detectable_list_debug_gate()
            .matches([Some(universal_frame), None])
            .then(|| self.original_static_creation_order(npc_id));
        if detectable_mutation_debug_owner_slot_matches(npc_id.index()) {
            let owner_creation_order = self.original_static_creation_order(npc_id);
            if detectable_mutation_debug_owner_matches(npc_id.index(), owner_creation_order) {
                let npc = self
                    .entities()
                    .get(npc_id)
                    .and_then(Entity::ai_actor_data)
                    .expect("DETMUT owner lost AI actor data before detection refresh");
                debug_detectable_mutation_snapshot(
                    "refresh_entry_snapshot",
                    "tick_enemy_ai_refresh_detection",
                    universal_frame,
                    npc_id,
                    owner_creation_order,
                    &npc.detectable_lists,
                    |target_id| Some(self.original_static_creation_order(target_id)),
                );
            }
        }
        if let Some(creation_order) = detectable_list_debug_creation_order
            && let Some(npc) = self.entities().get(npc_id).and_then(Entity::ai_actor_data)
        {
            debug_all_detectable_list_buckets(
                "optical_entry",
                npc_id,
                npc,
                universal_frame,
                creation_order,
            );
        }

        if self.optical_viewer_is_eligible(npc_id) {
            // The broad-phase box belongs to optical entry, while each
            // target's position and visibility parameters are queried live.
            let owner = self
                .entities()
                .expect_entity(npc_id, format_args!("optical entry"));
            let ground = owner.ground_position();
            let radius = owner
                .ai_actor_data()
                .expect("optical entry requires NPC data")
                .view_radius;
            let think_input = self.tick_enemy_ai_refresh_detection_for_npc(
                npc_id,
                tcx.assets,
                universal_frame,
                ground,
                radius,
            );
            // Enemy predetection may already have queued shadows. Append
            // the ordered Enemy VIEW / OUTOFVIEW block now, before later
            // detectable types, preserving the original
            // SHADOW → (VIEW|OUTOFVIEW)* → BODY → OBJECT → FRIEND →
            // MISSED_FRIEND → BEGGAR FIFO.
            if let Some(enemy_stimuli) = think_input {
                stimuli.extend(enemy_stimuli);
            }
            // The original NPC update completes this NPC's entire
            // detection-refresh scan before flushing its FIFO stimulus list.
            // Each per-type entry queries its current target before updating the
            // observer's latches. Think starts only after all buckets finish.
            self.tick_enemy_ai_refresh_per_type_for_npc(
                npc_id,
                tcx.assets,
                universal_frame,
                ground,
                radius,
                &mut stimuli,
            );
        }
        if let Some(creation_order) = detectable_list_debug_creation_order
            && let Some(npc) = self.entities().get(npc_id).and_then(Entity::ai_actor_data)
        {
            debug_all_detectable_list_buckets(
                "optical_exit",
                npc_id,
                npc,
                universal_frame,
                creation_order,
            );
        }

        self.dispatch_optical_stimuli(tcx, npc_id, stimuli);
    }

    /// Test seam: mutate entity/sequence state immediately before detection.
    #[cfg(test)]
    pub(crate) fn refresh_detection_after_live_mutation_for_test(
        &mut self,
        tcx: TickCtx<'_>,
        mutate_live_state: impl FnOnce(&mut Self),
    ) {
        mutate_live_state(self);
        let owners: Vec<_> = self.entities().ai_owner_ids().collect();
        for owner in owners {
            self.tick_enemy_ai_refresh_detection(tcx, owner);
        }
    }

    #[cfg(test)]
    pub(crate) fn enemy_optical_viewer_context_for_test(&self, npc_id: EntityId) -> bool {
        self.optical_viewer_is_eligible(npc_id)
    }

    fn optical_viewer_is_eligible(&self, npc_id: EntityId) -> bool {
        let Some(entity) = self.entities().get(npc_id) else {
            return false;
        };
        let Some(npc) = entity.ai_actor_data() else {
            return false;
        };
        if (!entity.is_active()
            && self
                .entity_building_sector(entity.element_data().sector())
                .is_none())
            || entity.is_dead()
            || entity.is_unconscious()
            || entity.element_data().posture() == Posture::Tied
        {
            return false;
        }
        match entity {
            Entity::Pc(_) => {
                npc.ai_brain.enemy().unwrap_or_else(|| {
                    panic!(
                        "eligible autonomous PC {} has no EnemyAi brain during detection",
                        npc_id.index()
                    )
                });
            }
            Entity::Soldier(_) => {
                npc.ai_brain.enemy().unwrap_or_else(|| {
                    panic!(
                        "eligible soldier NPC {} has no EnemyAi brain during detection",
                        npc_id.index()
                    )
                });
            }
            Entity::Civilian(_) => {
                npc.ai_brain.friendly().unwrap_or_else(|| {
                    panic!(
                        "eligible civilian NPC {} has no FriendlyAi brain during detection",
                        npc_id.index()
                    )
                });
            }
            _ => unreachable!("non-AI entity passed optical viewer gate"),
        }
        true
    }

    /// Scan Enemy entries in place. Optical stimuli remain queued until every
    /// detectable bucket has finished, because Think can change those lists.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    fn tick_enemy_ai_refresh_detection_for_npc(
        &mut self,
        npc_id: EntityId,
        assets: &LevelAssets,
        universal_frame: u32,
        ground: GroundPoint,
        radius: u16,
    ) -> Option<Vec<crate::ai::Stimulus>> {
        let mut stimuli = Vec::new();
        let creation_order = self.original_static_creation_order(npc_id);
        let modified_frame = refresh_detection_modified_frame(universal_frame, creation_order);
        let bucket = DetectableType::Enemy as usize;

        // Cleanup removes dead enemies only. Admission policy does not
        // retroactively remove an existing entry after a camp change.
        let mut index = 0;
        while index < self.ai_actor(npc_id, "Enemy cleanup").detectable_lists[bucket].len() {
            let target = self.ai_actor(npc_id, "Enemy cleanup").detectable_lists[bucket][index]
                .element
                .expect("Enemy detectable has no target");
            let entity = self.entities().expect_entity(
                target,
                format_args!("Enemy cleanup target for NPC {npc_id:?}"),
            );
            assert!(
                matches!(entity, Entity::Pc(_) | Entity::Soldier(_)),
                "Enemy detectable requires a PC or soldier"
            );
            if entity.is_dead() {
                self.ai_actor_mut(npc_id, "Enemy cleanup").detectable_lists[bucket].remove(index);
            } else {
                index += 1;
            }
        }
        let npc = self.ai_actor(npc_id, "Enemy scan");
        debug_detectable_list_entries(
            "post_cleanup",
            bucket,
            npc_id,
            &npc.detectable_lists[bucket],
            universal_frame,
            creation_order,
        );
        let count = npc.detectable_lists[bucket].len();
        let mut sum = 0u16;
        let mut observed_pcs = Vec::new();
        for index in 0..count {
            let entry = &self.ai_actor(npc_id, "Enemy scan").detectable_lists[bucket][index];
            let target = entry.element.expect("Enemy scan target");
            let owner = self
                .entities()
                .expect_entity(npc_id, format_args!("Enemy scan owner"));
            let inside = self.entity_data_inside_building(owner.element_data());
            let target_ground = self
                .entities()
                .expect_entity(target, format_args!("Enemy scan target"))
                .ground_position();
            if !refresh_detection_scans_target(
                entry.last_visibility,
                inside,
                ground,
                radius,
                target_ground,
            ) {
                let entry = &mut self
                    .ai_actor_mut(npc_id, "Enemy outer gate")
                    .detectable_lists[bucket][index];
                entry.seen_now = false;
                entry.last_visibility = 0.0;
                continue;
            }

            let visibility = self.enemy_detectable_visibility(
                assets,
                npc_id,
                index,
                universal_frame,
                modified_frame,
            );
            let entity = self
                .entities()
                .expect_entity(target, format_args!("Enemy predetection target"));
            let is_pc = matches!(entity, Entity::Pc(_));
            let guarded = matches!(entity, Entity::Pc(pc) if pc.pc.guard.is_some());
            let position = entity.element_data().position_map();
            let shadow_position = crate::ai::Position {
                x: position.x,
                y: position.y,
                sector: entity.element_data().sector(),
                level: entity.element_data().layer(),
            };
            let hostile = self.is_hostile_to_player_camp(
                self.entities()
                    .expect_entity(npc_id, format_args!("Enemy observer camp"))
                    .camp(),
            );
            let npc = self.ai_actor_mut(npc_id, "Enemy sharpness");
            let speed = if npc.view_lean_out {
                ai_vision::LOOK_DOWN_BASE_VIEW_SPEED
            } else {
                ai_vision::BASE_VIEW_SPEED
            };
            let sharpness = detection_sharpness(speed, visibility);
            let entry = &mut npc.detectable_lists[bucket][index];
            let shadow = update_predetection_shadow_latch(
                sharpness > 0,
                npc.detection_suspects[bucket],
                is_pc,
                guarded,
                &mut entry.shadow_seen_last_frame,
            );
            if !entry.seen_last_frame {
                sum = accumulate_detection_sharpness(sum, sharpness);
            }
            entry.seen_now = sharpness > 0;
            entry.last_visibility = visibility;
            let ai = npc
                .ai_brain
                .base_mut()
                .expect("optical owner requires controller");
            ai.max_visibility = ai.max_visibility.max(u32::from(sharpness));
            if shadow {
                stimuli.push(crate::ai::Stimulus::with_position(
                    crate::ai::StimulusType::EventSeesShadow,
                    shadow_position,
                ));
            }
            if achievement_observation_sample(sharpness > 0, is_pc, hostile) {
                observed_pcs.push(target);
            }
        }

        let entity = self
            .entities()
            .expect_entity(npc_id, format_args!("Enemy instant detection"));
        let aligned = self
            .mission_domain
            .diplomacy
            .is_player_aligned(entity.camp());
        let state = entity
            .ai_actor_data()
            .expect("optical owner requires NPC data")
            .ai_state();
        let instant = aligned
            || !matches!(
                state,
                crate::ai::AiState::Sleeping
                    | crate::ai::AiState::Default
                    | crate::ai::AiState::Wondering
            );
        let npc = self.ai_actor_mut(npc_id, "Enemy suspect");
        npc.detection_suspects[bucket] = npc.detection_suspects[bucket].wrapping_add(sum);
        if sum > 0 && (npc.worst_detected_type as usize) > bucket {
            npc.worst_detected_type = DetectableType::Enemy;
        }
        let committed = u32::from(npc.detection_suspects[bucket])
            >= ai_vision::DETECTION_SUSPECT_THRESHOLD
            || (instant && sum > 0);
        npc.detection_suspects[bucket] = if committed {
            0
        } else {
            cool_detection_suspect(sum, npc.detection_suspects[bucket], universal_frame)
        };
        npc.maximal_detection_suspect = npc.detection_suspects[bucket];

        for index in 0..count {
            let entry = &self.ai_actor(npc_id, "Enemy detection").detectable_lists[bucket][index];
            let target = entry.element.expect("Enemy detection target");
            let rising = committed && entry.seen_now && !entry.seen_last_frame;
            let falling = !entry.seen_now && entry.seen_last_frame;
            if rising {
                stimuli.push(crate::ai::Stimulus::with_human(
                    crate::ai::StimulusType::EventView,
                    target.index(),
                ));
                let entity = self
                    .entities()
                    .expect_entity(target, format_args!("Enemy reveal target"));
                if aligned && matches!(entity, Entity::Soldier(_)) && entity.element_data().blipped
                {
                    self.entities_mut()
                        .expect_entity_mut(target, format_args!("Enemy reveal target"))
                        .reveal_blip();
                }
            }
            if falling {
                stimuli.push(crate::ai::Stimulus::with_human(
                    crate::ai::StimulusType::EventOutOfView,
                    target.index(),
                ));
            }
            let entry = &mut self
                .ai_actor_mut(npc_id, "Enemy detection latch")
                .detectable_lists[bucket][index];
            if committed {
                entry.seen_last_frame = entry.seen_now;
            } else if falling {
                entry.seen_last_frame = false;
            }
        }
        for target in observed_pcs {
            self.record_achievement_hostile_observation(npc_id, target);
        }
        (!stimuli.is_empty()).then_some(stimuli)
    }

    fn enemy_detectable_visibility(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        index: usize,
        universal_frame: u32,
        modified_frame: u32,
    ) -> f32 {
        let viewer = self
            .entities()
            .expect_entity(owner, format_args!("Enemy visibility owner"));
        let npc = viewer
            .ai_actor_data()
            .expect("Enemy visibility requires NPC data");
        let ai = npc
            .ai_brain
            .base()
            .expect("Enemy visibility requires controller");
        if npc.eye_status.is_blind() {
            return 0.0;
        }
        let entry = &npc.detectable_lists[DetectableType::Enemy as usize][index];
        let target_id = entry.element.expect("Enemy visibility target");
        let seen_last = entry.seen_last_frame;
        let cached = entry.last_visibility;
        let target = self
            .entities()
            .expect_entity(target_id, format_args!("Enemy visibility target"));
        let hostile = self.is_hostile_to_player_camp(viewer.camp());
        let aligned = self
            .mission_domain
            .diplomacy
            .is_player_aligned(viewer.camp());
        let is_pc = matches!(target, Entity::Pc(_));
        if target.is_dead()
            || (hostile
                && target
                    .human_data()
                    .expect("Enemy target must be human")
                    .hollow_man)
        {
            return 0.0;
        }
        if hostile && is_pc {
            if matches!(target, Entity::Pc(pc) if !seen_last && pc.pc.guard.is_some()) {
                return 0.0;
            }
            if viewer.element_data().blipped
                && !self.entity_data_inside_building(viewer.element_data())
            {
                return 0.0;
            }
        }
        let frequency = if matches!(target, Entity::Soldier(_)) || aligned {
            ai_vision::DETECTION_FREQUENCY_ENEMY_NPC
        } else {
            ai_vision::DETECTION_FREQUENCY_ENEMY_PC
        };
        let refresh = modified_frame.is_multiple_of(frequency)
            || (hostile
                && lacklandist_visibility_refresh_always(npc.eye_status, ai.view_alert_status));
        let mut visibility = if refresh {
            let raw = self.live_human_visibility_for_detection(
                assets,
                owner,
                target_id,
                universal_frame,
                seen_last,
            );
            let speed = if hostile && is_pc {
                let Entity::Pc(pc) = self
                    .entities()
                    .expect_entity(target_id, format_args!("Enemy PC detection speed"))
                else {
                    unreachable!()
                };
                let profile = assets
                    .profile_manager
                    .get_character(pc.pc.profile_index)
                    .expect("optical PC requires character profile");
                0.01 * if self.world.weather.is_forest_level {
                    profile.detection_speed_in_forest
                } else {
                    profile.detection_speed_in_city
                } as f32
            } else {
                1.0
            };
            frequency as f32 * raw * speed
        } else {
            cached
        };
        // The learning flag is persistent controller state, updated at the
        // animation check before the next detectable is scanned.
        if hostile && is_pc {
            let order = self
                .entities()
                .current_element_for_actor(target_id)
                .and_then(|(sequence, index)| self.seq().get_element(sequence, index))
                .and_then(|element| element.current_order())
                .map_or(crate::order::OrderType::Invalid, |order| order.order_type);
            let ai = self.ai_mut(owner, "Enemy disguise learning");
            visibility = apply_enemy_beggar_disguise_with_relationship(
                hostile,
                true,
                &mut ai.got_the_beggar_trick,
                order,
                visibility,
            );
        }
        visibility
    }

    #[cfg(test)]
    pub(crate) fn enemy_optical_geometry_for_test(
        &mut self,
        _assets: &LevelAssets,
        target: EntityId,
    ) -> (crate::ai::Position, crate::coordinates::WorldPoint3D) {
        let entity = self
            .entities()
            .expect_entity(target, format_args!("test optical target"));
        assert!(!entity.is_dead(), "test optical target must be alive");
        let element = entity.element_data();
        let point = crate::stealth::detection_point_world(
            element.position(),
            element.posture(),
            element.direction(),
            entity.soldier_data().is_some_and(|soldier| soldier.rider),
        );
        (self.live_ai_position(target), point)
    }

    // ── P3c. Per-NPC non-Enemy detection (Body / Object /
    //         Friend / MissedFriend / Beggar) ────────────────────
    //
    // Per-`type` outer branches of detection refresh for every
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
    //  - Body: checks body-ignore state + `viewer_in_building`;
    //    visibility = `BODY_DETECTION_FACTOR * DETECTION_FREQUENCY_BODY
    //    * compute_visibility(body_as_human)`; `InstantDetection`
    //    rule `!matches!(state, Sleeping|Default|Wondering)`;
    //    rising-edge `EventSeesBody` + drop-on-commit; participates in
    //    `maximal_detection_suspect` (`type < FRIEND`);
    //    predetection shadow events for PC-typed bodies (the
    //    PC check effectively restricts shadow dispatch to
    //    PC bodies).
    //  - Object: gates on `viewer_in_building`; visibility =
    //    `DETECTION_FREQUENCY_OBJECT * compute_object_visibility(...)`;
    //    `InstantDetection` rule
    //    `!matches!(state, Sleeping|Default)` (note: Wondering is
    //    instant for Objects, unlike Body/Enemy);
    //    rising-edge `EventSeesObject` + drop-on-commit; participates
    //    in `maximal_detection_suspect`; inline detectable cleanup
    //    drops `!active` entries.  No shadow events —
    //    Predetection's PC gate skips objects
    //    unconditionally.
    //  - Friend: reject if unable to help or viewer is in a building;
    //    visibility = `DETECTION_FREQUENCY_FRIEND *
    //    compute_visibility(human)`; `InstantDetection` always
    //    true; rising-edge `EventSeesSoldier` + drop-on-commit; does
    //    NOT contribute to `maximal_detection_suspect`
    //    (`type < FRIEND`).  No shadow events.
    //  - MissedFriend: reject if dead, unconscious, or
    //    the viewer is in a building; visibility =
    //    `DETECTION_FREQUENCY_MISSED_FRIEND *
    //    compute_visibility(human)`; `InstantDetection` always
    //    true; rising-edge `EventSeesCharly` + drop-on-commit; does
    //    NOT contribute to `maximal_detection_suspect`.
    //  - Beggar: reject if dead, unconscious, or
    //    the viewer is in a building; visibility =
    //    `DETECTION_FREQUENCY_BEGGAR * compute_visibility(human)`;
    //    `InstantDetection` always true; rising-edge
    //    `EventSeesBeggar` + drop-on-commit; does NOT contribute to
    //    `maximal_detection_suspect`. Inline detectable cleanup
    //    drops entries whose target is no longer
    //    a genuine or disguised beggar.
    /// Per-NPC body of the non-enemy portion of detection refresh.
    /// One full iteration of the per-type loop body for
    /// `type ∈ {Body, Object, Friend, MissedFriend, Beggar}`.
    #[tracing::instrument(level = "trace", skip_all, fields(npc = npc_id.index()))]
    fn tick_enemy_ai_refresh_per_type_for_npc(
        &mut self,
        npc_id: EntityId,
        assets: &LevelAssets,
        universal_frame: u32,
        ground: GroundPoint,
        radius: u16,
        stimuli: &mut Vec<crate::ai::Stimulus>,
    ) {
        use crate::ai::{AiState, StimulusType, Substate as AiSubstate};
        use crate::element::Human as _;
        let creation_order = self.original_static_creation_order(npc_id);
        let modified_frame = refresh_detection_modified_frame(universal_frame, creation_order);
        for (kind, frequency, factor, event) in [
            (
                DetectableType::Body,
                ai_vision::DETECTION_FREQUENCY_BODY,
                3.0,
                StimulusType::EventSeesBody,
            ),
            (
                DetectableType::Object,
                ai_vision::DETECTION_FREQUENCY_OBJECT,
                1.0,
                StimulusType::EventSeesObject,
            ),
            (
                DetectableType::Friend,
                ai_vision::DETECTION_FREQUENCY_FRIEND,
                1.0,
                StimulusType::EventSeesSoldier,
            ),
            (
                DetectableType::MissedFriend,
                ai_vision::DETECTION_FREQUENCY_MISSED_FRIEND,
                1.0,
                StimulusType::EventSeesCharly,
            ),
            (
                DetectableType::Beggar,
                ai_vision::DETECTION_FREQUENCY_BEGGAR,
                1.0,
                StimulusType::EventSeesBeggar,
            ),
        ] {
            if kind == DetectableType::Object {
                Self::cleanup_live_detectables(self.entities_mut(), npc_id, kind, |entity| {
                    entity.is_active()
                });
            } else if kind == DetectableType::Beggar {
                Self::cleanup_live_detectables(
                    self.entities_mut(),
                    npc_id,
                    kind,
                    super::actor_queries::is_live_beggar,
                );
            }
            let bucket = kind as usize;
            let npc = self.ai_actor(npc_id, "optical bucket");
            debug_detectable_list_bucket(
                "post_cleanup",
                bucket,
                npc_id,
                npc,
                universal_frame,
                creation_order,
            );
            let count = npc.detectable_lists[bucket].len();
            let suspects_before = npc.detection_suspects[bucket];
            let gate_open = modified_frame.is_multiple_of(frequency);
            let mut sum = 0u16;
            let mut maximum = 0u32;
            let mut shadows = Vec::new();
            for index in 0..count {
                let npc = self.ai_actor(npc_id, "optical entry");
                let det = &npc.detectable_lists[bucket][index];
                let target_id = det.element.expect("optical detectable requires a target");
                let previous = det.last_visibility;
                let seen_last_frame = det.seen_last_frame;
                let owner = self
                    .entities()
                    .expect_entity(npc_id, format_args!("optical viewer"));
                let target = self
                    .entities()
                    .expect_entity(target_id, format_args!("optical target"));
                let npc = owner
                    .ai_actor_data()
                    .expect("optical viewer requires NPC data");
                let inside = self.entity_data_inside_building(owner.element_data());
                let in_building = self.entity_data_in_building_sector(owner.element_data());
                let scan = refresh_detection_scans_target(
                    previous,
                    inside,
                    ground,
                    radius,
                    target.ground_position(),
                );
                ai_vision::debug_view_radius_target_event(
                    "scan",
                    universal_frame,
                    npc_id,
                    bucket,
                    index,
                    target_id,
                    previous,
                    inside,
                    ground,
                    target.ground_position(),
                    radius,
                    scan,
                    gate_open,
                    None,
                    None,
                    None,
                    None,
                );
                let target_blocked = match kind {
                    DetectableType::Body => matches!(
                        npc.ai_substate(),
                        AiSubstate::SeekingOfficerWaitForAlertingSoldier
                            | AiSubstate::SeekingOfficerGetAlertingReportFromSoldier
                    ),
                    DetectableType::Friend => {
                        !matches!(target, Entity::Soldier(soldier) if soldier.is_able_to_help())
                    }
                    DetectableType::MissedFriend | DetectableType::Beggar => {
                        target.is_dead() || target.is_unconscious()
                    }
                    _ => false,
                };
                let blocked = non_enemy_visibility_blocked_with_relationship(
                    npc.eye_status,
                    self.is_hostile_to_player_camp(owner.camp()),
                    in_building || target_blocked,
                );
                let speed = if npc.view_lean_out {
                    ai_vision::LOOK_DOWN_BASE_VIEW_SPEED
                } else {
                    ai_vision::BASE_VIEW_SPEED
                };
                let visibility = if !scan || blocked {
                    0.0
                } else if !gate_open {
                    previous
                } else if kind == DetectableType::Object {
                    frequency as f32
                        * self.live_object_visibility_for_detection(assets, npc_id, target_id)
                } else {
                    factor
                        * frequency as f32
                        * self.live_human_visibility_for_detection(
                            assets,
                            npc_id,
                            target_id,
                            universal_frame,
                            seen_last_frame,
                        )
                };
                let sharpness = detection_sharpness(speed, visibility);
                maximum = maximum.max(u32::from(sharpness));
                if !seen_last_frame {
                    sum = accumulate_detection_sharpness(sum, sharpness);
                }
                let target = self
                    .entities()
                    .expect_entity(target_id, format_args!("optical predetection target"));
                let pc = matches!(target, Entity::Pc(_));
                let guarded = matches!(target, Entity::Pc(pc) if pc.pc.guard.is_some());
                let raw = target.element_data();
                let position = crate::ai::Position {
                    x: raw.position_map().x,
                    y: raw.position_map().y,
                    sector: raw.sector(),
                    level: raw.layer(),
                };
                let npc = self.ai_actor_mut(npc_id, "optical latches");
                let det = &mut npc.detectable_lists[bucket][index];
                if scan
                    && kind == DetectableType::Body
                    && update_predetection_shadow_latch(
                        sharpness > 0,
                        suspects_before,
                        pc,
                        guarded,
                        &mut det.shadow_seen_last_frame,
                    )
                {
                    shadows.push(position);
                }
                det.seen_now = sharpness > 0;
                det.last_visibility = visibility;
            }
            let npc = self.ai_actor_mut(npc_id, "optical commit");
            let state = npc
                .ai_brain
                .base()
                .expect("optical owner requires AI")
                .current_state;
            let instant = match kind {
                DetectableType::Body => !matches!(
                    state,
                    AiState::Sleeping | AiState::Default | AiState::Wondering
                ),
                DetectableType::Object => !matches!(state, AiState::Sleeping | AiState::Default),
                _ => true,
            };
            npc.detection_suspects[bucket] = suspects_before.wrapping_add(sum);
            if sum > 0 && npc.worst_detected_type as u8 > kind as u8 {
                npc.worst_detected_type = kind;
            }
            let mut rising = Vec::new();
            if npc.detection_suspects[bucket] >= ai_vision::DETECTION_SUSPECT_THRESHOLD as u16
                || (instant && sum > 0)
            {
                npc.detection_suspects[bucket] = 0;
                npc.detectable_lists[bucket].retain_mut(|det| {
                    let id = det.element.expect("optical commit requires target");
                    if det.seen_now && !det.seen_last_frame {
                        rising.push(id);
                        false
                    } else {
                        true
                    }
                });
            }
            npc.detection_suspects[bucket] =
                cool_detection_suspect(sum, npc.detection_suspects[bucket], universal_frame);
            if (kind as u8) < DetectableType::Friend as u8 {
                npc.maximal_detection_suspect = npc
                    .maximal_detection_suspect
                    .max(npc.detection_suspects[bucket]);
            }
            let ai = npc.ai_brain.base_mut().expect("optical commit requires AI");
            ai.max_visibility = ai.max_visibility.max(maximum);
            if kind == DetectableType::Object {
                for target in rising {
                    let mut stimulus = crate::ai::Stimulus::new(event);
                    stimulus.info = crate::ai::StimulusInfo::Object(
                        crate::ai::AiEntityHandle::new(target.index()),
                    );
                    stimuli.push(stimulus);
                }
            } else {
                stimuli.extend(queued_human_detection_stimuli(event, shadows, rising));
            }
        }
        finalize_detection_summary(self.ai_actor_mut(npc_id, "optical summary"));
    }

    fn cleanup_live_detectables(
        entities: &mut crate::entities::Entities,
        owner: EntityId,
        kind: DetectableType,
        keep: impl Fn(&Entity) -> bool,
    ) {
        let kind = kind as usize;
        let mut index = 0;
        loop {
            let entries = &entities
                .expect_ai_actor_data(owner, format_args!("detection cleanup"))
                .detectable_lists[kind];
            let Some(entry) = entries.get(index) else {
                break;
            };
            let retained = entry
                .element
                .and_then(|id| entities.get(id))
                .is_some_and(&keep);
            if retained {
                index += 1;
            } else {
                entities
                    .expect_ai_actor_data_mut(owner, format_args!("detection cleanup"))
                    .detectable_lists[kind]
                    .remove(index);
            }
        }
    }

    fn live_object_visibility_for_detection(
        &self,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
    ) -> f32 {
        let viewer = self
            .entities()
            .expect_entity(owner, format_args!("object visibility viewer"));
        let npc = viewer
            .ai_actor_data()
            .expect("object visibility requires NPC data");
        let target = self
            .entities()
            .expect_entity(target, format_args!("object visibility target"));
        let element = target.element_data();
        let (eye, eye_world) = human_eye_point_for_visibility(viewer);
        let mut target_world = element.position();
        target_world.z += 1.0;
        let obstacles = self.world.sight_obstacles(assets);
        ai_vision::compute_object_visibility(&ai_vision::ObjectVisibilityQuery {
            viewer_los: eye,
            viewer_world: eye_world,
            viewer_direction: viewer.element_data().direction(),
            view_forward: (npc.view_direction[0], npc.view_direction[1]),
            view_radius: npc.view_radius,
            viewer_eye_status: npc.eye_status,
            real_half_aperture: npc.real_half_aperture,
            viewer_in_building: self.entity_data_in_building_sector(viewer.element_data()),
            object_belongs_to_beggar: target
                .object_data()
                .expect("object detectable requires object data")
                .belongs_to_beggar,
            target_los: element.position_map(),
            target_world,
            sight_obstacles: obstacles,
            fast_grid: &self.world.fast_grid,
            layer: viewer.element_data().layer(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diplomacy::{DiplomacyDefinition, DiplomacyState};

    #[test]
    fn soldier_sight_uses_retained_world_cache_while_three_d_is_invalid() {
        // schema12 Savegame_SuN1Sh1nE/Profile_004/Savegame_034 replay-010,
        // frame 1581: Soldier 114 changes elevation during its hourglass.
        // Visibility uses the retained position despite its invalid cache
        // marker; resolving the new plane changes
        // every visibility origin and therefore accumulated detection.
        let raw_feet = crate::coordinates::WorldPoint3D::new(1898.5637, 836.4165, 0.594492);
        let mut soldier =
            crate::engine::test_support::actors::unbound_soldier(crate::element::Posture::Flying);
        soldier.element.set_position(raw_feet);
        let mut entity = Entity::Soldier(soldier);
        let resolved_map = crate::coordinates::MapPoint::new(1898.5637, 790.822);
        entity.position_iface_mut().set_map_position(resolved_map);
        let position = entity.position_iface_mut();
        let mut state = position.v48_serialized_state();
        state.position = raw_feet;
        state
            .computed_position
            .remove(crate::position_interface::PositionComputed::THREE_D);
        position.restore_v48_serialized_state(state);

        assert_eq!(entity.position_iface().get_position(), raw_feet);
        let (_, eye) = human_eye_point_for_visibility(&entity);
        assert_eq!(eye.x.to_bits(), raw_feet.x.to_bits());
        assert_eq!(eye.y.to_bits(), raw_feet.y.to_bits());
        assert_eq!(eye.z.to_bits(), (raw_feet.z + 45.0).to_bits());
    }

    #[test]
    fn optical_passing_door_follows_selected_command_without_runtime_door_state() {
        let mut engine = EngineInner::new();
        let target = engine.add_test_entity(Entity::Pc(
            crate::engine::test_support::actors::unbound_pc(crate::element::Posture::Upright),
        ));
        assert!(!selected_actor_is_passing_door(
            &engine.world.entities,
            &engine.orders.sequence_manager,
            target
        ));

        let sequence =
            engine
                .orders
                .sequence_manager
                .insert_element(crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::PassDoor,
                    Some(target),
                ));
        engine
            .orders
            .sequence_manager
            .start_sequence_level(sequence);
        engine.t_element_in_progress(&LevelAssets::new(), sequence, 0);

        engine.select_sequence_element(target, Some((sequence, 0)));
        assert!(selected_actor_is_passing_door(
            &engine.world.entities,
            &engine.orders.sequence_manager,
            target
        ));
    }

    #[test]
    fn ghost_observation_uses_live_non_royalist_player_allegiance() {
        let diplomacy = DiplomacyState::from_definition(
            true,
            true,
            Some(&DiplomacyDefinition {
                player_coalition: vec![4],
                relationships: vec![],
            }),
        )
        .expect("custom player coalition should be valid");

        assert!(achievement_observation_sample(
            true,
            true,
            diplomacy.is_hostile_to_player(Camp::Royalists),
        ));
        assert!(!achievement_observation_sample(
            true,
            true,
            diplomacy.is_hostile_to_player(Camp::Custom(4)),
        ));
    }

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
        let visibility = apply_enemy_beggar_disguise_with_relationship(
            Camp::Lacklandists.is_hostile_to(Camp::Royalists),
            true,
            &mut got_beggar_trick,
            crate::order::OrderType::SimulatingBeggar,
            16.0,
        );
        assert_eq!(visibility, 0.0);
        assert!(!got_beggar_trick);

        let visibility = apply_enemy_beggar_disguise_with_relationship(
            Camp::Lacklandists.is_hostile_to(Camp::Royalists),
            true,
            &mut got_beggar_trick,
            crate::order::OrderType::TransitionWaitingUprightSimulatingBeggar,
            16.0,
        );
        assert_eq!(visibility, 16.0);
        assert!(got_beggar_trick);
    }

    #[test]
    fn blind_type_gate_and_royalist_block_non_enemy_visibility_before_cadence() {
        assert!(non_enemy_visibility_blocked_with_relationship(
            crate::element::EyeStatus::Closed,
            Camp::Lacklandists.is_hostile_to(Camp::Royalists),
            false,
        ));
        assert!(non_enemy_visibility_blocked_with_relationship(
            crate::element::EyeStatus::LookForward,
            Camp::Lacklandists.is_hostile_to(Camp::Royalists),
            true,
        ));
        assert!(non_enemy_visibility_blocked_with_relationship(
            crate::element::EyeStatus::LookForward,
            Camp::Royalists.is_hostile_to(Camp::Royalists),
            false,
        ));
        assert!(!non_enemy_visibility_blocked_with_relationship(
            crate::element::EyeStatus::LookForward,
            Camp::Lacklandists.is_hostile_to(Camp::Royalists),
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
        // apart in the original game's ground-position Y. Their projected map Y differs
        // by 471.49 because the PC stands roughly 314 units lower. A map-space
        // broad phase incorrectly suppresses the visibility query entirely.
        let viewer_ground = GroundPoint::new(491.0, 1_135.001);
        let target_ground = GroundPoint::new(668.028_6, 1_292.568_2);
        assert!(refresh_detection_scans_target(
            0.0,
            false,
            viewer_ground,
            400,
            target_ground,
        ));

        let viewer_map = MapPoint::new(491.0, 715.0);
        let target_map = MapPoint::new(668.028_6, 1_186.492_8);
        let map_radius_y = 400.0 * crate::position_interface::ASPECT_RATIO;
        assert!((target_map.y - viewer_map.y).abs() > map_radius_y);
    }

    #[test]
    fn persistent_view_radius_distinguishes_owner_frame_and_surface_and_zero_is_a_miss() {
        let first = EntityId::from(crate::entity_id::SoldierId(7));
        let second = EntityId::from(crate::entity_id::SoldierId(9));
        let mut cache = crate::ai_vision::ViewRadiusCache::default();
        let surface = crate::position_interface::ObstacleHandle::new(3);
        cache.set(None, first, 40, 125.0);
        cache.set(surface, first, 40, 321.0);
        assert_eq!(cache.get(None, first, 40), Some(125.0));
        assert_eq!(cache.get(surface, first, 40), Some(321.0));
        assert_eq!(cache.get(surface, first, 41), None);
        assert_eq!(cache.get(surface, second, 40), None);
        cache.set(surface, second, 40, 0.0);
        assert_eq!(cache.get(surface, first, 40), None);
        assert_eq!(cache.get(surface, second, 40), None);
        assert_eq!(cache.get(None, first, 40), Some(125.0));
        cache.set(surface, second, 40, 90.0);
        assert_eq!(cache.get(surface, second, 40), Some(90.0));
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
        let soldier = WorldPoint3D::new(1079.0, 2_300.001, 150.001);
        assert!(listen_distance_squared(listener, soldier) < LIMIT_SQUARED);
        let projected_dy = (2150.0 - 2717.0) * INVERSE_ASPECT_RATIO;
        let old_projected_square = 18.0_f32.powi(2) + projected_dy.powi(2) + 150.001_f32.powi(2);
        assert!(old_projected_square >= LIMIT_SQUARED);

        // Leicester frame 402 exercises the opposite sign: map-only Y places
        // Civilian 74 inside the sphere, but positive elevation increases
        // world Y separation and Original correctly keeps it blipped.
        let listener = WorldPoint3D::new(1130.0, 248.0, 0.0);
        let civilian = WorldPoint3D::new(738.0, 740.001, 140.001);
        assert!(listen_distance_squared(listener, civilian) >= LIMIT_SQUARED);
        let projected_dy = (600.0 - 248.0) * INVERSE_ASPECT_RATIO;
        let old_projected_square = 392.0_f32.powi(2) + projected_dy.powi(2) + 140.001_f32.powi(2);
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
        let pc_eye = crate::coordinates::WorldPoint3D::new(1937.0, 1_604.001, 265.001);
        let blip_eye = crate::coordinates::WorldPoint3D::new(2_494.211_7, 1_623.488_9, 483.001);

        assert!(sees_blip_in_range(
            pc_eye,
            blip_eye,
            400.0,
            BLIP_SUPER_DETECTION,
        ));

        let pc_projected = MapPoint::from_world_xyz(pc_eye.x, pc_eye.y, 220.001);
        let blip_projected = MapPoint::from_world_xyz(blip_eye.x, blip_eye.y, 438.001);
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
        // reads the map position and keeps turning until the body itself is
        // within 50x30.
        let owner = MapPoint::new(635.0, 1414.0);
        let forecast_target = MapPoint::new(588.0, 1422.0);
        let literal_target = MapPoint::new(569.884_03, 1_423.860_4);

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
    fn forest_180_degree_view_depends_only_on_level_and_player_alignment() {
        assert!(forest_180_degree_view_enabled_with_relationship(true, true));
        assert!(!forest_180_degree_view_enabled_with_relationship(
            false, true
        ));
        assert!(!forest_180_degree_view_enabled_with_relationship(
            true, false
        ));
    }

    #[test]
    fn legendary_noise_sensitivity_applies_only_to_hostile_soldiers() {
        use crate::player_profile::DifficultyLevel;

        assert_eq!(
            difficulty_hearing_factor(true, DifficultyLevel::Legendary),
            1.5
        );
        assert_eq!(difficulty_hearing_factor(true, DifficultyLevel::Hard), 1.0);
        assert_eq!(
            difficulty_hearing_factor(false, DifficultyLevel::Legendary),
            1.0
        );
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
