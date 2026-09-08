//! Sprite visual variant (day / night / fog).
//!
//! Moved into engine (Decision 3C) so sim code can refer to it without
//! importing `robin_assets`. Asset loaders in `robin_assets::frame_holder`
//! re-export this type.

use serde::{Deserialize, Serialize};

/// Visual variant for sprite rendering (day, night, fog).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[repr(u32)]
pub enum SpriteVariant {
    Day = 0,
    Night = 1,
    Fog = 2,
}
