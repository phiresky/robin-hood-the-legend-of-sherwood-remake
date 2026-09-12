use super::*;

#[test]
fn current_door_seek_schema_requires_exact_sector_provenance_field() {
    let info = DoorSeekInfo {
        door_index: crate::gate::DoorIndex::new(1).unwrap(),
        door_type: crate::gate::DoorType::Default,
        point_out: MapPoint::new(1.0, 2.0),
        position_in: Position::default(),
        sector_out: 3,
        sector_out_index: crate::fast_find_grid::SectorIndex::new(4),
        sector_in: 5,
        layer_out: 6,
        npc_villain_authorized_direct: true,
    };
    let mut value = serde_json::to_value(info).unwrap();
    value.as_object_mut().unwrap().remove("sector_out_index");
    assert!(serde_json::from_value::<DoorSeekInfo>(value).is_err());
}
