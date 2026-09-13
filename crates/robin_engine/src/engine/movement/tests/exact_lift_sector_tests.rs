use super::*;
use crate::element::{ActorData, ActorPc, ElementData, ElementKind, Entity, HumanData, PcData};
use crate::fast_find_grid::{FastFindGrid, GridSector, SectorIndex};
use crate::sector::{LiftType, SectorNumber, SectorType};

fn sector(number: SectorNumber, lift: Option<LiftType>) -> GridSector {
    GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: lift.map_or(SectorType::AREA, |_| SectorType::LIFT),
        layer: 3,
        sector_number: number,
        door_index: None,
        lift_type: lift,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: lift.map(|_| MapPoint::new(2279.0, 1300.0)),
        high_exit_point: lift.map(|_| MapPoint::new(2279.0, 1200.0)),
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    }
}

fn pc126(sector: crate::position_interface::SectorHandle) -> Entity {
    let mut element = {
        let mut initial_element =
            ElementData::from_initial_posture(crate::element::Posture::OnWall);
        initial_element.kind = ElementKind::ActorPc;
        initial_element
    };
    element.set_position_map(MapPoint::new(2_278.88, 1257.0005));
    element.sprite.position_iface.set_sector_topology(
        crate::position_interface::SectorHandle::new(sector.get()),
        sector.arena_index(),
    );
    Entity::Pc(ActorPc {
        element,
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData::default(),
    })
}

#[test]
fn pc126_on_wall_uses_exact_duplicate_sector_for_climbing_down() {
    let public = SectorNumber::new(62);
    let ordinary_index = SectorIndex::new(0).unwrap();
    let wall_index = SectorIndex::new(1).unwrap();
    let mut grid = FastFindGrid::default();
    let level = std::sync::Arc::make_mut(&mut grid.level);
    level.sectors.push(sector(public, None));
    level.sectors.push(sector(public, Some(LiftType::Wall)));
    level
        .sector_number_map
        .insert(public, usize::from(ordinary_index));

    let exact_wall = crate::position_interface::SectorHandle::new(62)
        .unwrap()
        .with_arena_index(wall_index);
    assert_eq!(
        determine_lift_movement_animation_for(
            &pc126(exact_wall),
            &grid,
            crate::element::Posture::OnWall,
            OrderType::WalkingUpright,
            MapPoint::new(2279.0, 1269.0),
        ),
        OrderType::ClimbingWallDown,
        "Pc126's gate-73 approach must use the exact Wall sector's downward action"
    );

    let exact_ordinary = crate::position_interface::SectorHandle::new(62)
        .unwrap()
        .with_arena_index(ordinary_index);
    assert_eq!(
        determine_lift_movement_animation_for(
            &pc126(exact_ordinary),
            &grid,
            crate::element::Posture::OnWall,
            OrderType::WalkingUpright,
            MapPoint::new(2279.0, 1269.0),
        ),
        OrderType::WalkingUpright,
        "an exact ordinary duplicate must not acquire Wall movement"
    );

    assert_eq!(
        determine_lift_movement_animation_for(
            &pc126(crate::position_interface::SectorHandle::new(62).unwrap()),
            &grid,
            crate::element::Posture::OnWall,
            OrderType::WalkingUpright,
            MapPoint::new(2279.0, 1269.0),
        ),
        OrderType::WalkingUpright,
        "identity-less legacy positions retain the public-number fallback"
    );
}

#[test]
#[should_panic(expected = "sector 62 carries missing exact arena index 9")]
fn exact_sector_identity_never_falls_back_when_its_arena_object_is_missing() {
    let grid = FastFindGrid::default();
    let missing = crate::position_interface::SectorHandle::new(62)
        .unwrap()
        .with_arena_index(SectorIndex::new(9).unwrap());
    let _ = grid_sector_for_position_handle(&grid.level, missing);
}
