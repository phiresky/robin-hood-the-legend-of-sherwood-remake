use super::{current_door_for_route_source, sector_hits_have_distinct_identity};
use crate::element::{ActorData, ActorPc, ElementData, Entity, HumanData, PcData};
use crate::position_interface::DoorHandle;

fn pc_with_door_pass(crossed: bool) -> Entity {
    let mut entity = Entity::Pc(ActorPc {
        element: ElementData::default(),
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData::default(),
    });
    if !crossed {
        entity
            .position_iface_mut()
            .set_door(DoorHandle::new(53).expect("valid door index"), true);
    }
    entity
}

#[test]
fn route_source_uses_active_door_before_pass_callback() {
    let pc = pc_with_door_pass(false);

    assert_eq!(
        current_door_for_route_source(&pc),
        Some((DoorHandle::new(53).expect("valid door index"), true))
    );
}

#[test]
fn route_source_drops_active_door_after_pass_callback() {
    let pc = pc_with_door_pass(true);

    assert_eq!(current_door_for_route_source(&pc), None);
}

#[test]
fn route_source_does_not_resurrect_cleared_door() {
    let mut pc = pc_with_door_pass(false);
    pc.position_iface_mut().clear_door();

    assert_eq!(
        current_door_for_route_source(&pc),
        None,
        "the route source must respect the cleared position door reference"
    );
}

#[test]
fn route_source_uses_position_door_during_pass_callback_queue_window() {
    let mut pc = pc_with_door_pass(true);
    pc.position_iface_mut()
        .set_door(DoorHandle::new(17).expect("valid door index"), false);

    assert_eq!(
        current_door_for_route_source(&pc),
        Some((DoorHandle::new(17).expect("valid door index"), false))
    );
}

#[test]
fn goto_compares_sector_object_identity_even_when_numbers_match() {
    use crate::fast_find_grid::{SectorHit, SectorIndex};
    use crate::sector::SectorNumber;

    let number = SectorNumber::new(18);
    let hit = |index| SectorHit::Found {
        sector_idx: SectorIndex::new(index).unwrap(),
        sector_number: number,
    };

    let expected = crate::position_interface::SectorHandle::new(18).unwrap();
    assert!(sector_hits_have_distinct_identity(
        hit(12),
        hit(37),
        expected
    ));
    assert!(!sector_hits_have_distinct_identity(
        hit(12),
        hit(12),
        expected
    ));
    assert!(!sector_hits_have_distinct_identity(
        hit(12),
        SectorHit::None,
        expected
    ));
    assert!(!sector_hits_have_distinct_identity(
        hit(12),
        SectorHit::Found {
            sector_idx: SectorIndex::new(37).unwrap(),
            sector_number: SectorNumber::new(19),
        },
        expected
    ));
}
