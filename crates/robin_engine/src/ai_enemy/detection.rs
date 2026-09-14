use super::*;

/// Live viewer geometry for a forward-half-plane detection test.
pub(crate) struct Viewer180 {
    /// Identity the surface radius memo is keyed by — the ally when the
    /// test runs through an ally's eyes, not the deciding soldier.
    pub(crate) entity: crate::element::EntityId,
    pub(crate) eye_ground: crate::coordinates::GroundPoint,
    pub(crate) eye_z: f32,
    pub(crate) direction: u16,
    pub(crate) in_building: bool,
    pub(crate) view_radius: u16,
    pub(crate) sq_view_radius: f32,
    pub(crate) view_direction: [f32; 2],
    pub(crate) real_half_aperture: f32,
}

/// Live target geometry for a forward-half-plane detection test.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct Target180 {
    pub(crate) handle: HumanHandle,
    /// Raw active flag, not able-to-fight: an unconscious actor remains
    /// active and can still pass the 180-degree visibility test.
    pub(crate) active: bool,
    /// World-space detection point. Detection-point calculation starts from
    /// the raw element position; an AI-facing position may be a substituted
    /// door endpoint/carrier.
    pub(crate) detection_world: crate::coordinates::WorldPoint3D,
    /// Projection obstacle the target stands on (view-radius memo key).
    pub(crate) obstacle: Option<crate::position_interface::ObstacleHandle>,
}

/// Forward-half-plane detection with caller attribution for recorded queries.
#[track_caller]
pub(crate) fn detects_180_degrees_live(
    viewer: &Viewer180,
    target: &Target180,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    radius: impl FnOnce() -> f32,
) -> bool {
    // Step 1: viewer in a building — always returns false.
    if viewer.in_building {
        return false;
    }
    // Step 2: raw active flag of the target.
    if !target.active {
        return false;
    }

    let viewer_eye_z = viewer.eye_z;
    let target_detection_z = target.detection_world.z;
    let viewer_eye_ground = viewer.eye_ground;
    let target_detection_ground =
        crate::coordinates::GroundPoint::new(target.detection_world.x, target.detection_world.y);
    let target_handle = target.handle;

    // Aspect-ratio-stretched view vector (`INVERSE_ASPECT_RATIO`
    // on the Y component), from viewer eye to target detection point.
    let dx = target_detection_ground.x - viewer_eye_ground.x;
    let dy = (target_detection_ground.y - viewer_eye_ground.y)
        * crate::position_interface::INVERSE_ASPECT_RATIO;
    let sq_distance = dx * dx + dy * dy;
    tracing::trace!(
        target = target_handle,
        viewer_x = viewer_eye_ground.x,
        viewer_y = viewer_eye_ground.y,
        viewer_z = viewer_eye_z,
        target_x = target_detection_ground.x,
        target_y = target_detection_ground.y,
        sq_distance,
        sq_view_radius = viewer.sq_view_radius,
        "is_detecting_180_degrees: geometry"
    );
    if sq_distance > viewer.sq_view_radius {
        return false;
    }

    // Step 4: very-near "beside me" short-circuit; step 5: forward
    // half-plane (shared with the planar `detects_position_180_raw`).
    match half_plane_180(dx, dy, sq_distance, viewer.direction) {
        HalfPlane180::Beside => return true,
        HalfPlane180::NotBeside { forward_dot } => {
            if forward_dot < 0.0 {
                return false;
            }
        }
    }

    // Step 6: second, tighter radius gate against the spherical and
    // light-modulated radius. At night and in fog this is where the
    // viewer samples the surrounding shadow-light sectors, so it must
    // run for every target that survives the gates above — and only for
    // those, since the sampling is observable through the shared
    // per-surface radius cache.
    let effective_view_radius = radius();
    if sq_distance > effective_view_radius * effective_view_radius {
        return false;
    }

    crate::sight_obstacle::is_reachable_3d(
        sight_obstacles,
        [viewer_eye_ground.x, viewer_eye_ground.y, viewer_eye_z],
        [
            target_detection_ground.x,
            target_detection_ground.y,
            target_detection_z,
        ],
        crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
    )
}
