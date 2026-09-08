//! Ground-ale bottle spawn.
//!
//! The in-world bottle left behind by a `DropAle` command.  Distinct
//! from pre-placed mission bonuses of `BonusAle` type — those spawn
//! through the PARB chunk loader with the `BONUS_Ale` sprite, whereas
//! an ale dropped by a PC carries the *accessory* `ObjectType::Ale`
//! variant (sprite "ACCESSORIES_Ale", animation `OBJECT_LYING`).
//!
//! The `Command::DropAle` dispatcher lives in the action-dispatch
//! layer; this helper is the receiver that materialises the bottle.

use super::EngineInner;

impl EngineInner {}
