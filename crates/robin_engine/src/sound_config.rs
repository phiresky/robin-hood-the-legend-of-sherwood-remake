//! Sound volume and quality settings.
//!
//! The struct is `#[repr(C)]` so it can be shared across the C ABI; the
//! first seven fields preserve the original on-disk layout (integer
//! volumes 0–9 and legacy boolean flags).

use serde::{Deserialize, Serialize};

/// Per-profile sound settings.
///
/// Fields `music_volume` through `sound_8bit` preserve the original
/// on-disk layout.  Additional fields are appended for the Rust-side
/// feature set.
#[repr(C)]
#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SoundConfig {
    // --- ABI-compatible fields (must remain first, in this order) ---
    pub music_volume: u16,
    pub dialogue_volume: u16,
    pub fx_volume: u16,
    pub exclamation_volume: u16,
    pub amount_of_speaking: u16,
    pub sound_3d: bool,
    pub sound_8bit: bool,

    // --- Rust-only fields (appended after the legacy layout) ---
    pub master_volume: f32,
    pub music_muted: bool,
    pub fx_muted: bool,
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            music_volume: 9,
            dialogue_volume: 9,
            fx_volume: 9,
            exclamation_volume: 9,
            amount_of_speaking: 5,
            sound_3d: true,
            sound_8bit: false,
            master_volume: 1.0,
            music_muted: false,
            fx_muted: false,
        }
    }
}

impl SoundConfig {
    pub fn set_master_volume(&mut self, v: f32) {
        self.master_volume = v.clamp(0.0, 1.0);
    }

    pub fn set_music_volume(&mut self, v: u16) {
        self.music_volume = v.min(9);
    }

    pub fn set_fx_volume(&mut self, v: u16) {
        self.fx_volume = v.min(9);
    }

    pub fn mute_music(&mut self, muted: bool) {
        self.music_muted = muted;
    }

    pub fn mute_fx(&mut self, muted: bool) {
        self.fx_muted = muted;
    }

    pub fn is_music_muted(&self) -> bool {
        self.music_muted
    }

    pub fn is_fx_muted(&self) -> bool {
        self.fx_muted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let cfg = SoundConfig::default();
        assert_eq!(cfg.music_volume, 9);
        assert_eq!(cfg.dialogue_volume, 9);
        assert_eq!(cfg.fx_volume, 9);
        assert_eq!(cfg.exclamation_volume, 9);
        assert_eq!(cfg.amount_of_speaking, 5);
        assert!(cfg.sound_3d);
        assert!(!cfg.sound_8bit);
        assert_eq!(cfg.master_volume, 1.0);
        assert!(!cfg.music_muted);
        assert!(!cfg.fx_muted);
    }

    #[test]
    fn volume_clamping() {
        let mut cfg = SoundConfig::default();
        cfg.set_master_volume(1.5);
        assert_eq!(cfg.master_volume, 1.0);
        cfg.set_master_volume(-0.3);
        assert_eq!(cfg.master_volume, 0.0);

        cfg.set_music_volume(15);
        assert_eq!(cfg.music_volume, 9);
        cfg.set_fx_volume(5);
        assert_eq!(cfg.fx_volume, 5);
    }

    #[test]
    fn mute_toggles() {
        let mut cfg = SoundConfig::default();
        assert!(!cfg.is_music_muted());
        cfg.mute_music(true);
        assert!(cfg.is_music_muted());
        cfg.mute_music(false);
        assert!(!cfg.is_music_muted());

        assert!(!cfg.is_fx_muted());
        cfg.mute_fx(true);
        assert!(cfg.is_fx_muted());
    }

    #[test]
    fn serde_roundtrip() {
        let mut cfg = SoundConfig::default();
        cfg.set_master_volume(0.6);
        cfg.mute_fx(true);
        cfg.set_music_volume(5);

        let json = serde_json::to_string(&cfg).unwrap();
        let restored: SoundConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.master_volume, 0.6);
        assert_eq!(restored.music_volume, 5);
        assert!(restored.fx_muted);
        assert!(!restored.music_muted);
    }
}
