use super::Order;
use std::collections::VecDeque;

/// Canonical orders with stable queue-local identities. Insertion always creates
/// a new identity; mutation and movement within the queue retain it. Removed
/// identities are never reused, including after clearing the queue.
#[derive(
    Debug,
    Clone,
    Default,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct OrderQueue {
    orders: VecDeque<Order>,
    next_slot: u64,
}

impl OrderQueue {
    pub fn new() -> Self {
        Self::default()
    }

    fn allocate(&mut self, mut order: Order) -> Order {
        self.next_slot = self
            .next_slot
            .checked_add(1)
            .expect("order storage identity exhausted");
        order.storage_slot = self.next_slot;
        order
    }

    pub fn resolve(&self, slot: u64) -> Option<&Order> {
        self.orders.iter().find(|order| order.storage_slot == slot)
    }

    pub fn resolve_mut(&mut self, slot: u64) -> Option<&mut Order> {
        self.orders
            .iter_mut()
            .find(|order| order.storage_slot == slot)
    }

    pub fn push_back(&mut self, order: Order) {
        let order = self.allocate(order);
        self.orders.push_back(order);
    }

    pub fn insert(&mut self, index: usize, order: Order) {
        let order = self.allocate(order);
        self.orders.insert(index, order);
    }

    pub fn pop_front(&mut self) -> Option<Order> {
        self.orders.pop_front()
    }
    pub fn remove(&mut self, index: usize) -> Option<Order> {
        self.orders.remove(index)
    }
    pub fn clear(&mut self) {
        self.orders.clear();
    }
    pub fn truncate(&mut self, len: usize) {
        self.orders.truncate(len);
    }
    pub fn front_mut(&mut self) -> Option<&mut Order> {
        self.orders.front_mut()
    }
    pub fn back_mut(&mut self) -> Option<&mut Order> {
        self.orders.back_mut()
    }
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Order> {
        self.orders.get_mut(index)
    }
    pub fn iter_mut(&mut self) -> std::collections::vec_deque::IterMut<'_, Order> {
        self.orders.iter_mut()
    }
}

impl std::ops::Deref for OrderQueue {
    type Target = VecDeque<Order>;
    fn deref(&self) -> &Self::Target {
        &self.orders
    }
}

impl std::ops::IndexMut<usize> for OrderQueue {
    fn index_mut(&mut self, index: usize) -> &mut Order {
        &mut self.orders[index]
    }
}

impl std::ops::Index<usize> for OrderQueue {
    type Output = Order;
    fn index(&self, index: usize) -> &Order {
        &self.orders[index]
    }
}

impl Extend<Order> for OrderQueue {
    fn extend<T: IntoIterator<Item = Order>>(&mut self, orders: T) {
        for order in orders {
            self.push_back(order);
        }
    }
}

impl FromIterator<Order> for OrderQueue {
    fn from_iter<T: IntoIterator<Item = Order>>(orders: T) -> Self {
        let mut queue = Self::new();
        queue.extend(orders);
        queue
    }
}

impl From<Vec<Order>> for OrderQueue {
    fn from(orders: Vec<Order>) -> Self {
        orders.into_iter().collect()
    }
}

impl From<VecDeque<Order>> for OrderQueue {
    fn from(orders: VecDeque<Order>) -> Self {
        orders.into_iter().collect()
    }
}

impl IntoIterator for OrderQueue {
    type Item = Order;
    type IntoIter = std::collections::vec_deque::IntoIter<Order>;
    fn into_iter(self) -> Self::IntoIter {
        self.orders.into_iter()
    }
}

impl<'a> IntoIterator for &'a OrderQueue {
    type Item = &'a Order;
    type IntoIter = std::collections::vec_deque::Iter<'a, Order>;
    fn into_iter(self) -> Self::IntoIter {
        self.orders.iter()
    }
}

impl<'a> IntoIterator for &'a mut OrderQueue {
    type Item = &'a mut Order;
    type IntoIter = std::collections::vec_deque::IterMut<'a, Order>;
    fn into_iter(self) -> Self::IntoIter {
        self.orders.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::OrderType;

    fn order() -> Order {
        Order::test_new(OrderType::WaitingUprightBored, 0.0, 0.0)
    }

    #[test]
    fn installed_identity_survives_rewrite_and_insertion_before_it() {
        let mut queue = OrderQueue::new();
        queue.push_back(order());
        let slot = queue.front().unwrap().storage_slot;
        queue.front_mut().unwrap().order_type = OrderType::WaitingUprightBoredRandom;
        queue.front_mut().unwrap().order_id = std::num::NonZeroU32::new(999).unwrap();
        queue.insert(0, order());
        let installed = queue.resolve(slot).unwrap();
        assert_eq!(installed.order_type, OrderType::WaitingUprightBoredRandom);
        assert_eq!(installed.order_id.get(), 999);
        assert_ne!(queue.front().unwrap().storage_slot, slot);
    }

    #[test]
    fn removing_and_clearing_retire_slots_without_reuse() {
        let mut queue = OrderQueue::new();
        queue.push_back(order());
        let removed = queue.pop_front().unwrap();
        let first = removed.storage_slot;
        queue.push_back(removed);
        let second = queue.front().unwrap().storage_slot;
        assert_ne!(first, second);
        assert!(queue.resolve(first).is_none());
        queue.clear();
        queue.push_back(order());
        assert_ne!(queue.front().unwrap().storage_slot, second);
        assert!(queue.resolve(second).is_none());
    }

    #[test]
    fn snapshots_preserve_handles_and_keep_mutation_isolated() {
        let mut queue = OrderQueue::new();
        queue.push_back(order());
        let slot = queue.front().unwrap().storage_slot;
        let cloned = queue.clone();
        let json: OrderQueue =
            serde_json::from_str(&serde_json::to_string(&queue).unwrap()).unwrap();
        let binary: OrderQueue = bitcode::decode(&bitcode::encode(&queue)).unwrap();
        queue.resolve_mut(slot).unwrap().order_type = OrderType::WaitingUpright;
        for mut saved in [cloned, json, binary] {
            assert_eq!(
                saved.resolve(slot).unwrap().order_type,
                OrderType::WaitingUprightBored
            );
            saved.clear();
            saved.push_back(order());
            assert!(saved.resolve(slot).is_none());
        }
    }
}
