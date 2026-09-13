//! Sprite animation ambiance selection.

use serde::{Deserialize, Serialize};

/// Ambiance mode, used to pick the correct animation sub-directory.
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
pub enum Ambiance {
    Day = 1,
    Fog = 2,
    Night = 4,
    Attack = 8,
    Custom1 = 16,
    Custom2 = 32,
    Custom3 = 64,
    Custom4 = 128,
}

impl Ambiance {
    /// Sub-directory path fragment for this ambiance.
    pub fn directory_suffix(self) -> &'static str {
        match self {
            Ambiance::Day => "/Day/",
            Ambiance::Fog => "/Fog/",
            Ambiance::Night => "/Night/",
            Ambiance::Attack => "/Attack/",
            Ambiance::Custom1 => "/Custom1/",
            Ambiance::Custom2 => "/Custom2/",
            Ambiance::Custom3 => "/Custom3/",
            Ambiance::Custom4 => "/Custom4/",
        }
    }
}
