//! Shared mission ambiance and countdown configuration.

use serde::{Deserialize, Serialize};

/// Default view radius the engine hands out at level start before any
/// per-NPC mutation.  Used as the `view_radius` seed for freshly-spawned
/// NPCs.
pub const DEFAULT_VIEW_RADIUS: u16 = 400;

/// Reduced view radius for Fog/Night ambiances, installed at mission
/// load.
pub const NIGHT_VIEW_RADIUS: u16 = 300;

/// Level ambiance type (day, night, fog, etc.).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum Ambiance {
    #[serde(alias = "day", alias = "DAY")]
    #[default]
    Day,
    #[serde(alias = "fog", alias = "FOG")]
    Fog,
    #[serde(alias = "night", alias = "NIGHT")]
    Night,
    #[serde(alias = "attack", alias = "ATTACK")]
    Attack,
    #[serde(alias = "custom1", alias = "custom_1", alias = "CUSTOM_1")]
    Custom1,
    #[serde(alias = "custom2", alias = "custom_2", alias = "CUSTOM_2")]
    Custom2,
    #[serde(alias = "custom3", alias = "custom_3", alias = "CUSTOM_3")]
    Custom3,
    #[serde(alias = "custom4", alias = "custom_4", alias = "CUSTOM_4")]
    Custom4,
}

impl Ambiance {
    /// Map from the AMBIANCE_* integer constants.
    /// DAY=1, FOG=2, NIGHT=4, ATTACK=8, CUSTOM_1=16, CUSTOM_2=32,
    /// CUSTOM_3=64, CUSTOM_4=128. These are bitflags but only one is set.
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            1 => Ambiance::Day,
            2 => Ambiance::Fog,
            4 => Ambiance::Night,
            8 => Ambiance::Attack,
            16 => Ambiance::Custom1,
            32 => Ambiance::Custom2,
            64 => Ambiance::Custom3,
            128 => Ambiance::Custom4,
            _ => {
                tracing::warn!("Unknown ambiance value {}, defaulting to Day", raw);
                Ambiance::Day
            }
        }
    }

    /// Subdirectory name for map/minimap files.
    pub fn directory(&self) -> &'static str {
        match self {
            Ambiance::Day => "Day",
            Ambiance::Fog => "Fog",
            Ambiance::Night => "Night",
            Ambiance::Attack => "Attack",
            Ambiance::Custom1 => "Custom1",
            Ambiance::Custom2 => "Custom2",
            Ambiance::Custom3 => "Custom3",
            Ambiance::Custom4 => "Custom4",
        }
    }

    /// Convert to sprite_scriptor's Ambiance enum for .rhs file resolution.
    /// Attack/Custom_* use Day sprites (the shipping game has no dedicated
    /// sprite dictionaries for those ambiances — they reuse Day/Night art).
    pub fn to_sprite_ambiance(self) -> crate::sprite_ambiance::Ambiance {
        match self {
            Ambiance::Day
            | Ambiance::Attack
            | Ambiance::Custom1
            | Ambiance::Custom2
            | Ambiance::Custom3
            | Ambiance::Custom4 => crate::sprite_ambiance::Ambiance::Day,
            Ambiance::Fog => crate::sprite_ambiance::Ambiance::Fog,
            Ambiance::Night => crate::sprite_ambiance::Ambiance::Night,
        }
    }

    /// Convert to AMBIANCE_* bitmask for sound source filtering.
    /// DAY=1, FOG=2, NIGHT=4, ATTACK=8, CUSTOM_1..4=16/32/64/128.
    pub fn to_bitmask(self) -> u32 {
        match self {
            Ambiance::Day => 1,
            Ambiance::Fog => 2,
            Ambiance::Night => 4,
            Ambiance::Attack => 8,
            Ambiance::Custom1 => 16,
            Ambiance::Custom2 => 32,
            Ambiance::Custom3 => 64,
            Ambiance::Custom4 => 128,
        }
    }

    pub fn night_color_rgb(&self) -> (u8, u8, u8) {
        // The tint colour switch only lists Day/Fog/Night; the extra
        // ambiances fall through and are tinted like Day.
        match self {
            Ambiance::Day
            | Ambiance::Attack
            | Ambiance::Custom1
            | Ambiance::Custom2
            | Ambiance::Custom3
            | Ambiance::Custom4 => (45, 45, 35),
            Ambiance::Fog => (85, 77, 90),
            Ambiance::Night => (0, 0, 0),
        }
    }

    /// Initial `standard_view_polygon_radius` derived from the ambiance
    /// at header-load time. DAY / ATTACK / CUSTOM_1..4 default to the
    /// daytime view radius (400), FOG / NIGHT to the night view radius
    /// (300).
    pub fn default_view_polygon_radius(&self) -> u16 {
        match self {
            Ambiance::Fog | Ambiance::Night => NIGHT_VIEW_RADIUS,
            Ambiance::Day
            | Ambiance::Attack
            | Ambiance::Custom1
            | Ambiance::Custom2
            | Ambiance::Custom3
            | Ambiance::Custom4 => DEFAULT_VIEW_RADIUS,
        }
    }
}

/// Countdown visibility requested by a mission author.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
#[serde(rename_all = "snake_case")]
pub enum MissionCountdownMode {
    /// Keep the tracker visible throughout the active mission.
    #[default]
    Always,
    /// Show only once the authored warning threshold is reached.
    FinalOnly,
    /// Do not draw a tracker. Expiry still remains authoritative.
    Hidden,
}
