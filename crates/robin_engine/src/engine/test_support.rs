//! Shared fixtures and unwind-safe, observational test instrumentation.
pub(crate) mod actors;
pub(crate) mod asm;

/// Axis-aligned walkable test geometry, with stable counter-clockwise vertices.
pub(crate) fn square_sector(
    number: i16,
    layer: u16,
    min: crate::coordinates::MapPoint,
    max: crate::coordinates::MapPoint,
) -> crate::fast_find_grid::GridSector {
    use crate::coordinates::{MapBBox, MapPoint};
    crate::fast_find_grid::GridSector {
        points: vec![
            min,
            MapPoint::new(max.x, min.y),
            max,
            MapPoint::new(min.x, max.y),
        ],
        bounding_box: MapBBox::from_coords(min.x, min.y, max.x, max.y),
        sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        layer,
        sector_number: crate::sector::SectorNumber::new(number),
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    }
}

/// Supply explicit ordinary topology for fixtures that query only sector
/// metadata. No polygon is invented: geometric routing tests must install
/// their real geometry separately. Existing canonical sectors are preserved.
pub(crate) fn ensure_ordinary_sector(
    engine: &mut super::EngineInner,
    raw_sector: u16,
    layer: u16,
) -> crate::position_interface::SectorHandle {
    let number = crate::sector::SectorNumber::new(raw_sector as i16);
    let grid = engine.world.fast_grid_mut();
    if let Some(&index) = grid.level.sector_number_map.get(&number) {
        let sector = grid
            .level
            .sectors
            .get(index)
            .expect("fixture sector map must resolve");
        assert_eq!(sector.layer, layer, "fixture sector has a different layer");
    } else {
        grid.add_sector(
            crate::fast_find_grid::GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
                layer,
                sector_number: number,
                door_index: None,
                lift_type: None,
                lift_direction: 0,
                force_crouched: false,
                building_index: None,
                low_exit_point: None,
                high_exit_point: None,
                lowest_door_index: None,
                jump_line_indices: Vec::new(),
                gate_indices: Vec::new(),
                underlying_sector: None,
            },
            layer,
        );
    }
    crate::position_interface::SectorHandle::from_number(number)
}

pub(crate) use robin_test_support::probe::Probe;
