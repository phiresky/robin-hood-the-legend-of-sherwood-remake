use super::active_ability_order_type;
use crate::element::{ActorData, EntityId, EntityIdKind};
use crate::movement::AbilityKind;
use crate::order::OrderType;

#[test]
fn self_heal_keeps_the_canonical_healing_order_for_owner_selection() {
    let healer = EntityId::new(172, EntityIdKind::Pc);
    let mut actor = ActorData::default();
    actor.active_ability.kind = Some(AbilityKind::Heal);
    actor.active_ability.target = Some(healer);

    assert_eq!(active_ability_order_type(&actor), Some(OrderType::Healing));
}
