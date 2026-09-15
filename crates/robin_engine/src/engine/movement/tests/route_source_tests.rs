use super::{current_door_for_route_source, sector_hits_have_distinct_identity};
use crate::element::{
    ActiveDoorPass, ActorData, ActorPc, ElementData, Entity, HumanData, InstalledActorOrder, PcData,
};
use crate::gate::DoorIndex;
use crate::order::OrderType;
use crate::position_interface::DoorHandle;

fn pc_with_door_pass(triggers_fired: u8) -> Entity {
    pc_with_door_pass_directions(triggers_fired, true, true)
}

fn pc_with_door_pass_directions(triggers_fired: u8, direct: bool, position_direct: bool) -> Entity {
    let mut entity = Entity::Pc(ActorPc {
        element: ElementData::default(),
        actor: ActorData {
            active_door_pass: Some(ActiveDoorPass {
                door_index: DoorIndex::new(53).expect("valid door index"),
                direct,
                position_direct,
                triggers_fired,
            }),
            installed_order: Some(InstalledActorOrder {
                order_id: std::num::NonZeroU32::new(1).unwrap(),
                order_type: OrderType::WalkingUpright,
            }),
            ..ActorData::default()
        },
        human: HumanData::default(),
        pc: PcData::default(),
    });
    if triggers_fired == 0 {
        entity
            .position_iface_mut()
            .set_door(DoorHandle::new(53).expect("valid door index"), direct);
    }
    entity
}

#[test]
fn route_source_uses_active_door_before_pass_callback() {
    let pc = pc_with_door_pass(0);

    assert_eq!(
        current_door_for_route_source(&pc),
        Some((DoorHandle::new(53).expect("valid door index"), true))
    );
}

#[test]
fn route_source_drops_active_door_after_pass_callback() {
    let pc = pc_with_door_pass(1);

    assert_eq!(current_door_for_route_source(&pc), None);
}

#[test]
fn route_source_does_not_resurrect_postponed_door_under_unrelated_order() {
    let mut pc = pc_with_door_pass(0);
    pc.position_iface_mut().clear_door();
    pc.actor_data_mut().unwrap().installed_order = Some(InstalledActorOrder {
        order_id: std::num::NonZeroU32::new(2).unwrap(),
        order_type: OrderType::WaitingUpright,
    });

    assert_eq!(
        current_door_for_route_source(&pc),
        None,
        "the dormant pass mirror must not replace an absent position door reference"
    );
}

#[test]
fn route_source_reports_live_traversal_direction_not_element_direction() {
    // The original game reads the door-direction field written from the
    // live sector-side test at
    // launch. A v48-restored movement element can carry a different
    // serialized direction; that value belongs to
    // AI position state, not to route sourcing.
    let pc = pc_with_door_pass_directions(0, true, false);

    assert_eq!(
        current_door_for_route_source(&pc),
        Some((DoorHandle::new(53).expect("valid door index"), true))
    );
}

#[test]
fn route_source_uses_position_door_during_pass_callback_queue_window() {
    let mut pc = pc_with_door_pass(1);
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
