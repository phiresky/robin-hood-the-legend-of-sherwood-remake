use crate::ai::Position;
use crate::coordinates::WorldPoint3D;

/// Shared patrol list policy, independent of when/how the caller observes actors.
///
/// Admission runs once, in authored order, before any ordering. The callback
/// returns (admitted, alive); rejected living actors retain authored order in the
/// missed list. Callers retain their distinct LOS/state predicate order and chief
/// assignment timing. Geometry supplies separate distance and pairing positions:
/// a door-passing actor's raw world position is not its AI gate-endpoint position.
pub(super) fn assemble_patrol<T>(
    members: impl IntoIterator<Item = T>,
    mut admission: impl FnMut(&T) -> (bool, bool),
    geometry: impl Fn(&T) -> (WorldPoint3D, Position),
    chief_world: WorldPoint3D,
    chief_position: Position,
) -> (Vec<T>, Vec<T>) {
    let mut admitted = Vec::new();
    let mut missed = Vec::new();
    for member in members {
        let (include, alive) = admission(&member);
        if include {
            admitted.push(member);
        } else if alive {
            missed.push(member);
        }
    }

    let square_distance = |member: &T| {
        let (world, _) = geometry(member);
        let dx = world.x - chief_world.x;
        let dy = (world.y - chief_world.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
        let dz = world.z - chief_world.z;
        dx * dx + dy * dy + dz * dz
    };
    let mut sorted = Vec::with_capacity(admitted.len());
    for member in admitted {
        let distance = square_distance(&member);
        let insert_at = sorted
            .iter()
            .position(|existing| {
                super::patrol_distance_inserts_before(distance, square_distance(existing))
            })
            .unwrap_or(sorted.len());
        sorted.insert(insert_at, member);
    }
    for pair_end in (1..sorted.len()).step_by(2) {
        let (_, even) = geometry(&sorted[pair_end - 1]);
        let (_, odd) = geometry(&sorted[pair_end]);
        let ex = even.x - chief_position.x;
        let ey = even.y - chief_position.y;
        let ox = odd.x - chief_position.x;
        let oy = odd.y - chief_position.y;
        if ex * oy - ey * ox < 0.0 {
            sorted.swap(pair_end - 1, pair_end);
        }
    }
    (sorted, missed)
}

/// Reconstruct world Y only for paths whose observation is projected AI geometry.
pub(super) fn projected_patrol_world(position: Position, ground_z: f32) -> WorldPoint3D {
    WorldPoint3D::new(position.x, position.y + ground_z, ground_z)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(x: f32, y: f32) -> Position {
        Position {
            x,
            y,
            ..Default::default()
        }
    }

    #[test]
    fn ties_insert_first_and_odd_member_is_not_paired() {
        let positions = [position(1.0, 0.0), position(1.0, 0.0), position(5.0, 0.0)];
        let (patrol, missed) = assemble_patrol(
            0..3,
            |_| (true, true),
            |&id| (projected_patrol_world(positions[id], 0.0), positions[id]),
            WorldPoint3D::ZERO,
            Position::default(),
        );
        assert_eq!(patrol, [1, 0, 2]);
        assert!(missed.is_empty());
    }

    #[test]
    fn pairs_use_ai_positions_but_distance_uses_raw_door_positions() {
        // The gate endpoint is far away, but the actor's raw position is near
        // the chief. Elevation participates even when the map Y cancels it.
        let world = [
            WorldPoint3D::new(1.0, 0.0, 0.0),
            WorldPoint3D::new(2.0, 0.0, 0.0),
            WorldPoint3D::new(0.0, 0.0, 10.0),
        ];
        let ai = [
            position(100.0, 0.0),
            position(0.0, -1.0),
            position(0.0, -10.0),
        ];
        let (patrol, _) = assemble_patrol(
            0..3,
            |_| (true, true),
            |&id| (world[id], ai[id]),
            WorldPoint3D::ZERO,
            Position::default(),
        );
        assert_eq!(
            patrol,
            [1, 0, 2],
            "near raw actors pair and negative determinant swaps them"
        );
    }

    #[test]
    fn projected_elevation_and_aspect_ratio_match_equivalent_world_observations() {
        let chief = position(20.0, 30.0);
        let chief_z = 10.0;
        let projected = [
            (position(21.0, 30.0), 10.0),
            (position(20.0, 29.0), 11.0),
            (position(20.0, 31.0), 10.0),
        ];
        let world = [
            WorldPoint3D::new(21.0, 40.0, 10.0),
            WorldPoint3D::new(20.0, 40.0, 11.0),
            WorldPoint3D::new(20.0, 41.0, 10.0),
        ];
        let projected_result = assemble_patrol(
            0..3,
            |_| (true, true),
            |&id| {
                let (position, z) = projected[id];
                (projected_patrol_world(position, z), position)
            },
            projected_patrol_world(chief, chief_z),
            chief,
        );
        let raw_result = assemble_patrol(
            0..3,
            |_| (true, true),
            |&id| (world[id], projected[id].0),
            WorldPoint3D::new(20.0, 40.0, 10.0),
            chief,
        );
        assert_eq!(projected_result, raw_result);
        assert_eq!(
            raw_result.0,
            [1, 0, 2],
            "equal X/Z distance precedes stretched world Y"
        );
    }

    #[test]
    fn admission_preserves_authored_los_order_and_missed_members_before_geometry() {
        use crate::ai::AiState;
        use std::cell::RefCell;
        // Active wrong-state actors still issue LOS; inactive ones do not.
        let states = [
            AiState::Attacking,
            AiState::Default,
            AiState::Default,
            AiState::Default,
        ];
        let active = [true, false, true, false];
        let alive = [true, true, true, false];
        let events = RefCell::new(Vec::new());
        let (patrol, missed) = assemble_patrol(
            0..4,
            |&id| {
                events.borrow_mut().push(("admission", id));
                let admit = super::super::patrol_member_admitted(
                    active[id],
                    || {
                        events.borrow_mut().push(("los", id));
                        true
                    },
                    states[id],
                    false,
                    true,
                );
                (admit, alive[id])
            },
            |&id| {
                events.borrow_mut().push(("geometry", id));
                (WorldPoint3D::ZERO, Position::default())
            },
            WorldPoint3D::ZERO,
            Position::default(),
        );
        assert_eq!(patrol, [2]);
        assert_eq!(missed, [0, 1]);
        assert_eq!(
            *events.borrow(),
            [
                ("admission", 0),
                ("los", 0),
                ("admission", 1),
                ("admission", 2),
                ("los", 2),
                ("admission", 3),
                ("geometry", 2)
            ]
        );
    }

    #[test]
    fn empty_all_rejected_and_unordered_distances_keep_insertion_contract() {
        let geometry = |&id: &usize| {
            (
                WorldPoint3D::new(if id == 2 { f32::NAN } else { id as f32 }, 0.0, 0.0),
                Position::default(),
            )
        };
        let (patrol, missed) = assemble_patrol(
            0..0,
            |_| (true, true),
            geometry,
            WorldPoint3D::ZERO,
            Position::default(),
        );
        assert!(patrol.is_empty() && missed.is_empty());
        let (patrol, missed) = assemble_patrol(
            0..3,
            |_| (false, true),
            geometry,
            WorldPoint3D::ZERO,
            Position::default(),
        );
        assert!(patrol.is_empty());
        assert_eq!(missed, [0, 1, 2]);
        let (patrol, _) = assemble_patrol(
            0..3,
            |_| (true, true),
            geometry,
            WorldPoint3D::ZERO,
            Position::default(),
        );
        assert_eq!(patrol, [2, 0, 1]);
    }
}
