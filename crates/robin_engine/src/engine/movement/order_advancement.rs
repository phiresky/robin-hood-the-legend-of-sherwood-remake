//! Movement order advancement with sequence-only mutation authority.
//!
//! This module cannot reach entities, scripts, spatial state, feedback, or RNG.
//! The engine coordinator owns the synchronous `do_next_order` barrier between
//! preparing a selected order and classifying its terminal result.

use crate::element::EntityId;
use crate::order::OrderType;
use crate::sequence::{SequenceId, SequenceManager, SequenceState};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(super) struct SelectedMovementOrder {
    pub owner: EntityId,
    sequence_id: SequenceId,
    element_index: usize,
    final_order_will_exhaust: bool,
    order_id: std::num::NonZeroU32,
    order_type: OrderType,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PreparedMovementPop {
    selected: SelectedMovementOrder,
    live_following_before_pop: Vec<(SequenceId, usize)>,
}

/// Preserve the diagnostic owner even when a synchronous callback has already
/// replaced this captured movement as the actor's selected element.
pub(super) fn capture_selection(
    manager: &SequenceManager,
    sequence_id: SequenceId,
    element_index: usize,
) -> Option<(EntityId, Option<SelectedMovementOrder>)> {
    let element = manager.get_element(sequence_id, element_index)?;
    let owner = element.owner?;
    let selected = (element.state == SequenceState::InProgress
        && element.data.is_movement()
        && manager.current_element_for_actor(owner) == Some((sequence_id, element_index)))
    .then(|| {
        let order = element
            .current_order()
            .expect("selected movement has no current order");
        SelectedMovementOrder {
            owner,
            sequence_id,
            element_index,
            final_order_will_exhaust: element.orders.len() == 1,
            order_id: order.order_id,
            order_type: order.order_type,
        }
    });
    Some((owner, selected))
}

impl SelectedMovementOrder {
    /// Invalidate outgoing goal snapshots before the coordinator synchronously
    /// promotes a postponed replacement. Snapshot the live successor chain now,
    /// not after that callback is allowed to mutate it.
    pub(super) fn prepare(self, manager: &mut SequenceManager) -> PreparedMovementPop {
        let mut live_following_before_pop = Vec::new();
        if self.final_order_will_exhaust {
            manager.clear_retained_movement_goals_for_actor(self.owner);
            let mut cursor = (self.sequence_id, self.element_index);
            let mut visited = Vec::new();
            while let Some(following_ref) = manager.following_element_ref(cursor.0, cursor.1) {
                assert!(
                    !visited.contains(&following_ref),
                    "cycle in terminal movement chain rooted at {:?}:{}",
                    self.sequence_id,
                    self.element_index
                );
                visited.push(following_ref);
                let following = manager
                    .get_element(following_ref.0, following_ref.1)
                    .unwrap_or_else(|| {
                        panic!(
                            "terminal movement following element {:?}:{} disappeared",
                            following_ref.0, following_ref.1
                        )
                    });
                if matches!(
                    following.state,
                    SequenceState::Todo | SequenceState::Postponed | SequenceState::InProgress
                ) {
                    live_following_before_pop.push(following_ref);
                }
                cursor = following_ref;
            }
        }
        PreparedMovementPop {
            selected: self,
            live_following_before_pop,
        }
    }
}

impl PreparedMovementPop {
    /// Re-read the captured element after synchronous advancement. A callback
    /// may have removed it or replaced the live selection in the meantime.
    pub(super) fn finish(
        self,
        manager: &SequenceManager,
    ) -> Option<super::TerminalMovementOrderPop> {
        let selected = self.selected;
        (selected.final_order_will_exhaust
            && manager
                .get_element(selected.sequence_id, selected.element_index)
                .is_some_and(|element| {
                    element.owner == Some(selected.owner)
                        && element.data.is_movement()
                        && element.state == SequenceState::Terminated
                }))
        .then_some(super::TerminalMovementOrderPop {
            owner: selected.owner,
            sequence_id: selected.sequence_id,
            element_index: selected.element_index,
            order_id: selected.order_id,
            order_type: selected.order_type,
            live_following_before_pop: self.live_following_before_pop,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::Command;
    use crate::sequence::{Sequence, SequenceElement};

    fn selected_move() -> (SequenceManager, EntityId, SequenceId) {
        let owner = EntityId::Pc(crate::entity_id::PcId(7));
        let mut manager = SequenceManager::new();
        let mut sequence = Sequence::new();
        sequence.append_element(SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingUpright,
        ));
        let id = manager.launch_sequence(sequence);
        (manager, owner, id)
    }

    #[test]
    fn preparation_does_not_advance_selected_order() {
        let (mut manager, owner, id) = selected_move();
        let before = manager
            .get_element(id, 0)
            .unwrap()
            .current_order()
            .unwrap()
            .order_id;
        let (diagnostic_owner, selected) = capture_selection(&manager, id, 0).unwrap();
        assert_eq!(diagnostic_owner, owner);
        let prepared = selected.unwrap().prepare(&mut manager);
        assert_eq!(
            manager
                .get_element(id, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_id,
            before
        );
        assert!(
            prepared.finish(&manager).is_none(),
            "only the coordinator may advance the order"
        );
    }

    #[test]
    fn post_callback_classification_retains_captured_order_identity() {
        let (mut manager, owner, id) = selected_move();
        let selected = capture_selection(&manager, id, 0).unwrap().1.unwrap();
        let order_id = selected.order_id;
        let prepared = selected.prepare(&mut manager);
        manager.element_terminated(id, 0);
        let terminal = prepared.finish(&manager).expect("captured move terminated");
        assert_eq!(terminal.owner, owner);
        assert_eq!(terminal.order_id, order_id);
        assert_eq!(terminal.order_type, OrderType::WalkingUpright);
    }

    #[test]
    fn terminal_capture_cannot_mutate_a_replacement_selection() {
        let (mut manager, owner, id) = selected_move();
        manager.element_terminated(id, 0);
        let (diagnostic_owner, selected) = capture_selection(&manager, id, 0).unwrap();
        assert_eq!(diagnostic_owner, owner);
        assert!(selected.is_none());
        assert!(capture_selection(&manager, SequenceId(u32::MAX), 0).is_none());
    }
}
