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
/// balance derivation.  This table deliberately covers every production type
/// that maps to an inventory bonus, including the unfilled purse item.
pub const TRADE_ITEMS: [TradeItemDefinition; 9] = [
    TradeItemDefinition {
        prod_type: Type::MakeArrow,
        unit_price: 1,
    },
    TradeItemDefinition {
        prod_type: Type::MakePurse,
        unit_price: 2,
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
    /// Host-assigned session-local correlation id carried through the
    /// deterministic command stream. It prevents a delayed multiplayer
    /// acknowledgement from a closed panel being mistaken for a later sale.
    pub request_id: u64,
    pub prod_type: Type,
    pub quantity: TradeQuantity,
    pub outcome: TradeOutcome,
}

impl TradeReceipt {
    pub const fn rejected(
        request_id: u64,
        prod_type: Type,
        quantity: TradeQuantity,
        reason: TradeRejectReason,
    ) -> Self {
        Self {
            request_id,
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
    fn price_table_is_positive_and_unique() {
        let mut seen = HashSet::new();
        for item in TRADE_ITEMS {
            assert!(item.unit_price > 0);
            assert!(seen.insert(item.prod_type), "duplicate price entry");
        }
        assert_eq!(seen.len(), 9);
    }

    #[test]
    fn every_shipped_make_inventory_type_is_sellable() {
        let inventory_types: Vec<_> = (0..=12)
            .map(|raw| Type::from_script_i32(raw).expect("shipped production type ordinal"))
            .filter(|prod_type| prod_type.bonus_raw_type().is_some())
            .collect();

        assert_eq!(inventory_types.len(), 9, "original MAKE_* inventory count");
        assert_eq!(TRADE_ITEMS.len(), inventory_types.len());
        for prod_type in inventory_types {
            assert!(
                trade_item(prod_type).is_some(),
                "missing sell price for inventory production type {prod_type:?}"
            );
        }
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
                (Type::MakePurse, 2),
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

    #[test]
    fn shipped_full_stock_cannot_replace_campaign_ransom_progression() {
        // Authored Sherwood capacities, in the same order as TRADE_ITEMS.
        const CAPACITIES: [u16; 9] = [50, 25, 25, 25, 25, 25, 35, 15, 15];
        let full_stock_proceeds: u32 = TRADE_ITEMS
            .iter()
            .zip(CAPACITIES)
            .map(|(item, capacity)| u32::from(item.unit_price) * u32::from(capacity))
            .sum();

        assert_eq!(full_stock_proceeds, 815);
        assert!(
            full_stock_proceeds < 1_000,
            "selling every authored storage slot must remain supplementary income"
        );
        assert!(
            TRADE_ITEMS
                .iter()
                .all(|item| item.unit_price * TradeQuantity::Five.units() < 50),
            "one Sell 5 action must remain below a £50 beggar payment"
        );
    }
}
