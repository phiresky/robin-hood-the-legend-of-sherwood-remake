use super::*;
use crate::element::Command;
use crate::order::OrderType;
use crate::sequence::{Sequence, SequenceElement};

#[test]
fn same_sector_arrival_speech_follows_move_instead_of_running_in_parallel() {
    let owner = EntityId::Pc(crate::entity_id::PcId(7));
    let mut sequence = Sequence::new();
    sequence.append_element(SequenceElement::new_movement(
        1,
        Command::Move,
        Some(owner),
        OrderType::WalkingUpright,
    ));

    append_arrival_speech(&mut sequence, owner);

    assert_eq!(sequence.elements.len(), 2);
    assert_eq!(sequence.elements[0].command_level, 1);
    assert_eq!(
        sequence.elements[1].command,
        Command::SpeakHeroReachDestination
    );
    assert_eq!(
        sequence.elements[1].command_level, 2,
        "arrival speech must wait for the pathfinding Move to finish"
    );
}
