//! Deterministic Sherwood production-item trading.
//!
//! Prices are intentionally static simulation data.  The UI reads this table
//! and the authoritative command handler uses the same entries, so displayed
//! proceeds cannot drift from the amount credited to the campaign.

use serde::{Deserialize, Serialize};

use crate::sector_production::Type;

/// Quantities exposed by the trading screen.  Deliberately excludes a
/// state-dependent "sell all" operation: the command records the exact
/// requested quantity for replay and network determinism.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum TradeQuantity {
    One,
    Five,
}

impl TradeQuantity {
    pub const fn units(self) -> u16 {
        match self {
            Self::One => 1,
            Self::Five => 5,
        }
    }
}

/// One immutable entry in the Sherwood economy table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeItemDefinition {
    pub prod_type: Type,
    pub unit_price: u16,
}

/// Sellable production items in UI order.
///
/// All shipped MAKE sectors have the same authored speed (5), so labor/time
/// establishes a common floor.  Capacity scarcity and tactical utility then
/// distinguish the prices; see `docs/SHERWOOD_TRADING.md` for the complete
/// balance derivation.  Purses are excluded because their gameplay effect is
/// currency and selling them would form a direct money loop.
pub const TRADE_ITEMS: [TradeItemDefinition; 8] = [
    TradeItemDefinition {
        prod_type: Type::MakeArrow,
        unit_price: 1,
    },
    TradeItemDefinition {
        prod_type: Type::MakeStone,
        unit_price: 2,
    },
    TradeItemDefinition {
        prod_type: Type::MakeApple,
        unit_price: 3,
    },
    TradeItemDefinition {
        prod_type: Type::MakeLamblegg,
        unit_price: 3,
    },
    TradeItemDefinition {
        prod_type: Type::MakeAle,
        unit_price: 4,
    },
    TradeItemDefinition {
        prod_type: Type::MakePlant,
        unit_price: 5,
    },
    TradeItemDefinition {
        prod_type: Type::MakeNet,
        unit_price: 7,
    },
    TradeItemDefinition {
        prod_type: Type::MakeWaspNest,
        unit_price: 9,
    },
];

pub fn trade_item(prod_type: Type) -> Option<&'static TradeItemDefinition> {
    TRADE_ITEMS
        .iter()
        .find(|definition| definition.prod_type == prod_type)
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum TradeRejectReason {
    TradingDisabled,
    HostOnly,
    NotInSherwood,
    UnsupportedProductionType,
    InsufficientStock { available: u16 },
    CurrencyOverflow,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum TradeOutcome {
    Sold {
        units: u16,
        unit_price: u16,
        total_price: u16,
        remaining_stock: u16,
        ransom_after: i32,
    },
    Rejected(TradeRejectReason),
}

/// Authoritative feedback for one trade command.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct TradeReceipt {
    pub prod_type: Type,
    pub quantity: TradeQuantity,
    pub outcome: TradeOutcome,
}

impl TradeReceipt {
    pub const fn rejected(
        prod_type: Type,
        quantity: TradeQuantity,
        reason: TradeRejectReason,
    ) -> Self {
        Self {
            prod_type,
            quantity,
            outcome: TradeOutcome::Rejected(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn price_table_is_positive_unique_and_excludes_money_loops() {
        let mut seen = HashSet::new();
        for item in TRADE_ITEMS {
            assert!(item.unit_price > 0);
            assert!(seen.insert(item.prod_type), "duplicate price entry");
        }
        assert_eq!(seen.len(), 8);
        assert!(trade_item(Type::MakePurse).is_none());
    }

    #[test]
    fn documented_price_table_is_stable() {
        let actual: Vec<_> = TRADE_ITEMS
            .iter()
            .map(|item| (item.prod_type, item.unit_price))
            .collect();
        assert_eq!(
            actual,
            vec![
                (Type::MakeArrow, 1),
                (Type::MakeStone, 2),
                (Type::MakeApple, 3),
                (Type::MakeLamblegg, 3),
                (Type::MakeAle, 4),
                (Type::MakePlant, 5),
                (Type::MakeNet, 7),
                (Type::MakeWaspNest, 9),
            ]
        );
    }

    #[test]
    fn non_inventory_sectors_are_rejected_explicitly() {
        for prod_type in [
            Type::MakePurse,
            Type::TrainBow,
            Type::TrainHandToHand,
            Type::Heal,
            Type::Relic,
            Type::Unknown,
        ] {
            assert!(trade_item(prod_type).is_none(), "{prod_type:?}");
        }
    }

    #[test]
    fn price_order_reflects_capacity_and_tactical_power() {
        let price = |kind| trade_item(kind).unwrap().unit_price;
        assert!(price(Type::MakeArrow) < price(Type::MakeApple));
        assert!(price(Type::MakeApple) < price(Type::MakePlant));
        assert!(price(Type::MakePlant) < price(Type::MakeNet));
        assert!(price(Type::MakeNet) < price(Type::MakeWaspNest));
    }
}
