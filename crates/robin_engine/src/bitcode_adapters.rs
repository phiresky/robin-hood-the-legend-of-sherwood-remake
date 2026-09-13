//! Native codec adapters shared with the engine's stable value types.
pub use robin_engine_types::bitcode_adapters::*;

#[cfg(test)]
mod tests {
    #[test]
    fn nominal_indices_retain_niches_scalar_bytes_and_hashes() {
        use crate::patch::PatchIndex;
        use crate::sector::BuildingIdx;
        use robin_util::state_hash::compute;
        assert_eq!(std::mem::size_of::<Option<PatchIndex>>(), 4);
        assert_eq!(std::mem::size_of::<Option<BuildingIdx>>(), 2);
        assert!(PatchIndex::new(u32::MAX).is_none());
        assert!(BuildingIdx::new(u16::MAX).is_none());
        for value in [0, 1, 256, u32::MAX - 1] {
            let index = PatchIndex::new(value).unwrap();
            assert_eq!(bitcode::encode(&index), bitcode::encode(&value));
            assert_eq!(
                compute(&index),
                compute(&nonmax::NonMaxU32::new(value).unwrap())
            );
            assert_eq!(
                bitcode::decode::<PatchIndex>(&bitcode::encode(&index)).unwrap(),
                index
            );
            assert_eq!(
                serde_json::to_value(index).unwrap(),
                serde_json::json!(value)
            );
            assert_eq!(index.to_string(), value.to_string());
        }
    }

    #[test]
    fn flags_keep_unknown_bits_and_hash_their_underlying_scalar() {
        use crate::position_interface::PositionComputed;
        for bits in 0..=u8::MAX {
            let flags = PositionComputed::from_bits_retain(bits);
            assert_eq!(
                robin_util::state_hash::compute(&flags),
                robin_util::state_hash::compute(&bits)
            );
            assert_eq!(bitcode::encode(&flags), bitcode::encode(&bits));
            let restored: PositionComputed = bitcode::decode(&bitcode::encode(&flags)).unwrap();
            assert_eq!(restored.bits(), bits);
        }
    }
}
