use crate::campaign::CampaignValue;
use crate::entities::Entities;
use crate::entity_id::BonusId;
use crate::player_command::PlayerId;
use crate::profiles::{Action, MissionLocation};
use crate::sector_production::Type;
use crate::sound::{Jingle, SoundCommand};
use crate::trading::{TradeOutcome, TradeQuantity, TradeReceipt, TradeRejectReason, trade_item};

use super::{EngineInner, LevelAssets};

fn stored_stock(entities: &Entities, action: Action) -> u16 {
    let total: u32 = entities
        .bonuses()
        .filter(|(_, bonus)| bonus.element.active && bonus.object.associated_action == action)
        .map(|(_, bonus)| u32::from(bonus.object.quantity))
        .sum();
    u16::try_from(total)
        .expect("Sherwood stored-item stock exceeds the campaign u16 representation")
}

/// Remove exact units from active production-point bonus stacks in stable
/// element-table order.  The caller validates aggregate stock first, making a
/// short removal an invariant violation rather than a partial/fake sale.
fn remove_stored_units(entities: &mut Entities, action: Action, quantity: u16) {
    let ids: Vec<BonusId> = entities
        .bonuses()
        .filter(|(_, bonus)| bonus.element.active && bonus.object.associated_action == action)
        .map(|(id, _)| id)
        .collect();
    let mut remaining = quantity;
    for id in ids {
        if remaining == 0 {
            break;
        }
        let bonus = entities
            .get_bonus_mut(id)
            .expect("collected Sherwood storage bonus disappeared during one command");
        let removed = remaining.min(bonus.object.quantity);
        bonus.object.quantity -= removed;
        remaining -= removed;
        if bonus.object.quantity == 0 {
            bonus.element.active = false;
        }
    }
    assert_eq!(
        remaining, 0,
        "validated Sherwood stock could not satisfy an exact sale"
    );
}

impl EngineInner {
    pub(super) fn sell_sherwood_production_item(
        &mut self,
        assets: &LevelAssets,
        seat: usize,
        prod_type: Type,
        quantity: TradeQuantity,
    ) {
        let reject = |engine: &mut Self, reason| {
            engine
                .feedback
                .pending_side_effects
                .trade_receipts
                .push(TradeReceipt::rejected(prod_type, quantity, reason));
        };

        if seat != usize::from(PlayerId::HOST.0) {
            reject(self, TradeRejectReason::HostOnly);
            return;
        }
        if !self.control.sim_config.sherwood_trading {
            reject(self, TradeRejectReason::TradingDisabled);
            return;
        }
        let in_sherwood = self
            .mission_domain
            .campaign
            .current_mission_idx
            .and_then(|idx| self.mission_domain.campaign.missions.get(idx))
            .map(|mission| mission.profile(&assets.profile_manager).location)
            == Some(MissionLocation::Sherwood);
        if !in_sherwood {
            reject(self, TradeRejectReason::NotInSherwood);
            return;
        }

        let Some(definition) = trade_item(prod_type) else {
            reject(self, TradeRejectReason::UnsupportedProductionType);
            return;
        };
        let Some(action) = self
            .mission_domain
            .campaign
            .production_sectors
            .iter()
            .find(|sector| sector.prod_type == prod_type)
            .and_then(|sector| sector.associated_action())
        else {
            // A table entry without a matching item sector is programmer/data
            // corruption; returning "unsupported" would conceal the drift.
            panic!("tradable production type {prod_type:?} has no item sector/action")
        };

        let available = stored_stock(&self.world.entities, action);
        let units = quantity.units();
        if available < units {
            reject(self, TradeRejectReason::InsufficientStock { available });
            return;
        }
        let total_price = definition
            .unit_price
            .checked_mul(units)
            .expect("bounded Sherwood price table overflowed u16");
        let ransom_before = self
            .mission_domain
            .campaign
            .get_value(CampaignValue::Ransom);
        let Some(ransom_after) = ransom_before.checked_add(i32::from(total_price)) else {
            reject(self, TradeRejectReason::CurrencyOverflow);
            return;
        };

        remove_stored_units(&mut self.world.entities, action, units);
        // Deliberately bypass `add_campaign_value`: trade proceeds are
        // campaign currency, not mission-collected ransom or score/achievement
        // credit.  Preserve only the positive-cash acknowledgement jingle.
        self.mission_domain
            .campaign
            .set_value(CampaignValue::Ransom, ransom_after);
        if self.control.frame_counter > 0 {
            self.feedback
                .pending_side_effects
                .sounds
                .push(SoundCommand::Jingle(Jingle::CashWon));
        }

        self.feedback
            .pending_side_effects
            .trade_receipts
            .push(TradeReceipt {
                prod_type,
                quantity,
                outcome: TradeOutcome::Sold {
                    units,
                    unit_price: definition.unit_price,
                    total_price,
                    remaining_stock: available - units,
                    ransom_after,
                },
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::campaign::Campaign;
    use crate::element::{ElementBonus, ElementData, ElementKind, Entity, ObjectData, ObjectType};
    use crate::mission::Mission;
    use crate::profiles::{MissionProfile, ProfileManager};
    use std::sync::Arc;

    fn bonus(action: Action, quantity: u16, active: bool) -> Entity {
        Entity::Bonus(ElementBonus {
            element: ElementData {
                kind: ElementKind::ObjectBonus,
                active,
                ..Default::default()
            },
            object: ObjectData {
                object_type: ObjectType::BonusArrow,
                associated_action: action,
                quantity,
                ..Default::default()
            },
        })
    }

    #[test]
    fn exact_removal_crosses_stacks_and_deactivates_empty_stacks() {
        let mut entities = Entities::new();
        entities.push(Some(bonus(Action::Bow, 2, true)));
        entities.push(Some(bonus(Action::Bow, 4, true)));
        entities.push(Some(bonus(Action::Heal, 8, true)));

        assert_eq!(stored_stock(&entities, Action::Bow), 6);
        remove_stored_units(&mut entities, Action::Bow, 5);
        assert_eq!(stored_stock(&entities, Action::Bow), 1);
        assert_eq!(stored_stock(&entities, Action::Heal), 8);
        let bow: Vec<_> = entities
            .bonuses()
            .filter(|(_, item)| item.object.associated_action == Action::Bow)
            .map(|(_, item)| (item.element.active, item.object.quantity))
            .collect();
        assert_eq!(bow, vec![(false, 0), (true, 1)]);
    }

    #[test]
    fn inactive_and_unrelated_world_items_are_not_stored_stock() {
        let mut entities = Entities::new();
        entities.push(Some(bonus(Action::Bow, 5, false)));
        entities.push(Some(bonus(Action::Stone, 5, true)));
        assert_eq!(stored_stock(&entities, Action::Bow), 0);
    }

    fn sherwood_engine() -> (EngineInner, LevelAssets) {
        let mut profiles = ProfileManager::default();
        profiles.missions.push(MissionProfile {
            location: MissionLocation::Sherwood,
            ..MissionProfile::default()
        });
        let mut mission = Mission::new();
        mission.profile_idx = Some(0);
        let mut campaign = Campaign::default();
        campaign.missions.push(mission);
        campaign.current_mission_idx = Some(0);
        let mut engine = EngineInner::new();
        engine.mission_domain.campaign = campaign;
        engine
            .world
            .entities
            .push(Some(bonus(Action::Bow, 6, true)));
        let assets = LevelAssets {
            profile_manager: Arc::new(profiles),
            ..LevelAssets::default()
        };
        (engine, assets)
    }

    #[test]
    fn successful_sale_is_exact_and_does_not_credit_mission_money() {
        let (mut engine, assets) = sherwood_engine();
        let ransom_before = engine
            .mission_domain
            .campaign
            .get_value(CampaignValue::Ransom);
        engine.sell_sherwood_production_item(&assets, 0, Type::MakeArrow, TradeQuantity::Five);

        assert_eq!(stored_stock(&engine.world.entities, Action::Bow), 1);
        assert_eq!(
            engine
                .mission_domain
                .campaign
                .get_value(CampaignValue::Ransom),
            ransom_before + 5
        );
        assert_eq!(engine.mission_domain.mission_stat.collected_money, 0);
        assert_eq!(
            engine.feedback.pending_side_effects.trade_receipts,
            vec![TradeReceipt {
                prod_type: Type::MakeArrow,
                quantity: TradeQuantity::Five,
                outcome: TradeOutcome::Sold {
                    units: 5,
                    unit_price: 1,
                    total_price: 5,
                    remaining_stock: 1,
                    ransom_after: ransom_before + 5,
                },
            }]
        );
    }

    #[test]
    fn invalid_requests_reject_without_partial_mutation() {
        let (mut engine, assets) = sherwood_engine();
        let ransom_before = engine
            .mission_domain
            .campaign
            .get_value(CampaignValue::Ransom);

        engine.sell_sherwood_production_item(&assets, 1, Type::MakeArrow, TradeQuantity::One);
        engine.sell_sherwood_production_item(&assets, 0, Type::MakePurse, TradeQuantity::One);
        engine.sell_sherwood_production_item(&assets, 0, Type::MakeArrow, TradeQuantity::Five);
        engine.sell_sherwood_production_item(&assets, 0, Type::MakeArrow, TradeQuantity::Five);

        assert_eq!(stored_stock(&engine.world.entities, Action::Bow), 1);
        assert_eq!(
            engine
                .mission_domain
                .campaign
                .get_value(CampaignValue::Ransom),
            ransom_before + 5
        );
        let receipts = &engine.feedback.pending_side_effects.trade_receipts;
        assert_eq!(
            receipts[0].outcome,
            TradeOutcome::Rejected(TradeRejectReason::HostOnly)
        );
        assert_eq!(
            receipts[1].outcome,
            TradeOutcome::Rejected(TradeRejectReason::UnsupportedProductionType)
        );
        assert_eq!(
            receipts[3].outcome,
            TradeOutcome::Rejected(TradeRejectReason::InsufficientStock { available: 1 })
        );
    }
}
