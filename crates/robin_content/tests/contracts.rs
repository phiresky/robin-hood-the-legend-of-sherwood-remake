use robin_content::{PixelOpacityLookup, SpriteVariant};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
struct TwoPixels;

impl PixelOpacityLookup for TwoPixels {
    fn sprite_dimensions(&self, bank_id: u32) -> Option<(u16, u16)> {
        (bank_id == 7).then_some((2, 1))
    }

    fn is_pixel_opaque(&self, bank_id: u32, x: u16, y: u16, shadows: bool) -> bool {
        assert_eq!(bank_id, 7);
        assert!(x < 2 && y == 0);
        x == 0 || shadows
    }
}

#[test]
fn opacity_fingerprint_preserves_canonical_byte_layout() {
    let mut expected = Sha256::new();
    expected.update(b"robinhood-sprite-opacity-v1\0");
    expected.update(7_u32.to_le_bytes());
    expected.update(2_u16.to_le_bytes());
    expected.update(1_u16.to_le_bytes());
    // Each shadow interpretation starts a fresh packed byte, LSB first.
    expected.update([0, 0b01, 1, 0b11]);
    assert_eq!(
        TwoPixels.simulation_opacity_sha256(&[7]),
        <[u8; 32]>::from(expected.finalize()),
    );
}

#[test]
#[should_panic(expected = "simulation-reachable sprite bank id 8 is missing")]
fn missing_reachable_sprite_is_an_error_not_transparent_pixels() {
    TwoPixels.simulation_opacity_sha256(&[8]);
}

#[test]
fn variant_json_names_and_discriminants_are_unchanged() {
    for (variant, name, discriminant) in [
        (SpriteVariant::Day, "Day", 0),
        (SpriteVariant::Night, "Night", 1),
        (SpriteVariant::Fog, "Fog", 2),
    ] {
        assert_eq!(variant as u32, discriminant);
        assert_eq!(
            serde_json::to_string(&variant).unwrap(),
            format!("\"{name}\"")
        );
        assert_eq!(
            serde_json::from_str::<SpriteVariant>(&format!("\"{name}\"")).unwrap(),
            variant
        );
    }
}

#[cfg(feature = "simulation-codecs")]
#[test]
fn simulation_hash_and_snapshot_representation_are_unchanged() {
    use robin_util::state_hash::StateHash;
    use std::hash::{DefaultHasher, Hasher};
    for (index, variant) in [SpriteVariant::Day, SpriteVariant::Night, SpriteVariant::Fog]
        .into_iter()
        .enumerate()
    {
        let mut expected = DefaultHasher::new();
        expected.write_u64(index as u64);
        let mut actual = DefaultHasher::new();
        variant.state_hash(&mut actual);
        assert_eq!(actual.finish(), expected.finish());
        let encoded = bitcode::encode(&variant);
        assert_eq!(encoded, [index as u8]);
        assert_eq!(bitcode::decode::<SpriteVariant>(&encoded).unwrap(), variant);
    }
}
