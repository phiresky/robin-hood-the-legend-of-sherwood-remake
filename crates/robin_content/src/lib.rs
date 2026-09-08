//! Shared sprite-content contracts, independent of simulation and asset storage.

use serde::{Deserialize, Serialize};

/// Visual variant for sprite rendering (day, night, fog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(
    feature = "simulation-codecs",
    derive(robin_state_hash_derive::StateHash, bitcode::Encode, bitcode::Decode)
)]
#[repr(u32)]
pub enum SpriteVariant {
    Day = 0,
    Night = 1,
    Fog = 2,
}

/// Host-side callback for per-pixel sprite opacity.
///
/// Wired at level-load time into the engine's level assets: the host
/// owns the `FrameHolder` with the packed sprite banks and implements
/// this trait. The engine uses it to close the per-pixel sprite pick
/// path (transparent-color and dictionary-owned shadow rejection) without
/// depending on `robin_assets`.
pub trait PixelOpacityLookup: Send + Sync {
    /// Return the native pixel dimensions of one sprite-bank frame.
    ///
    /// Original's render path writes the current frame's dimensions back to
    /// the serialized sprite fields while creating its target surface.
    /// The Rust renderer deliberately cannot mutate simulation state, so
    /// parity diagnostics query the same immutable frame metadata here.
    fn sprite_dimensions(&self, _bank_id: u32) -> Option<(u16, u16)> {
        None
    }

    /// Return `true` if the pixel at local `(x, y)` within the sprite
    /// frame identified by `bank_id` is opaque.
    ///
    /// The lookup owns the exact dictionary generation and its bound shadow
    /// color. Shadow pixels are transparent unless `blue_pixels_are_in` is
    /// `true` (the engine passes the entity's `is_blipped` flag so blipped
    /// entities remain clickable in their shadow area).
    fn is_pixel_opaque(&self, bank_id: u32, x: u16, y: u16, blue_pixels_are_in: bool) -> bool;

    /// SHA-256 of the exact opacity behavior for the sorted reachable bank
    /// IDs. Implementations hash dimensions and both shadow interpretations,
    /// never storage-level dictionary/RLE bytes.
    fn simulation_opacity_sha256(&self, sorted_bank_ids: &[u32]) -> [u8; 32] {
        use sha2::{Digest as _, Sha256};

        let mut hash = Sha256::new();
        hash.update(b"robinhood-sprite-opacity-v1\0");
        for &bank_id in sorted_bank_ids {
            let (width, height) = self.sprite_dimensions(bank_id).unwrap_or_else(|| {
                panic!("simulation-reachable sprite bank id {bank_id} is missing")
            });
            hash.update(bank_id.to_le_bytes());
            hash.update(width.to_le_bytes());
            hash.update(height.to_le_bytes());
            for blue_pixels_are_in in [false, true] {
                hash.update([u8::from(blue_pixels_are_in)]);
                let mut byte = 0_u8;
                let mut bit = 0_u8;
                for y in 0..height {
                    for x in 0..width {
                        if self.is_pixel_opaque(bank_id, x, y, blue_pixels_are_in) {
                            byte |= 1 << bit;
                        }
                        bit += 1;
                        if bit == 8 {
                            hash.update([byte]);
                            byte = 0;
                            bit = 0;
                        }
                    }
                }
                if bit != 0 {
                    hash.update([byte]);
                }
            }
        }
        hash.finalize().into()
    }
}
