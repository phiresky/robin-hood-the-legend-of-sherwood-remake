//! Frozen v8 layout: runtime preparation must never change serialized field order.
use super::*;

#[test]
fn aggregate_budget_rejects_small_parts_forming_an_oversized_bank() {
    let sprite = ShippingSprite {
        width: 1024,
        height: 1024,
        dictionary_index: UNMAPPED_DICT,
        packed_data: Arc::new(vec![]),
        raster: None,
    };
    let mut bank = ShippingSpriteBank {
        signature: 1,
        dictionaries: vec![],
        sprite_count: 513,
        sprites: (0..513).map(|id| (id, sprite.clone())).collect(),
        vq_chunks: vec![],
        rle_jxl_chunks: vec![],
    };
    assert!(
        bank.validate_resident_budget()
            .unwrap_err()
            .to_string()
            .contains("resident bytes")
    );
    bank.sprites.truncate(1);
    bank.validate_resident_budget().unwrap();
    bank.sprite_count = u32::MAX;
    assert!(bank.validate_resident_budget().is_err());
}
#[derive(Default, Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct FrozenMissionV8 {
    pub levels: BTreeMap<String, LoadedLevel>,
    pub scripts: BTreeMap<String, ScbFile>,
    pub rhs_files: BTreeMap<String, RhsData>,
    pub sprite_bank: Option<ShippingSpriteBank>,
    pub raw: BTreeMap<String, Vec<u8>>,
    /// Exact durations from the source assets, keyed like `raw`.
    ///
    /// Web shipping may transcode WAV/Vorbis to Opus. Simulation timing must
    /// continue to use the authoritative source duration rather than codec
    /// delay, resampling, or a browser decoder's rounded duration.
    pub audio_durations_ms: BTreeMap<String, u32>,
}

#[test]
fn v8_payload_matches_frozen_wire_and_preparation_is_consuming() {
    let mut payload = ShippingMissionPayload::default();
    payload.scripts.insert(
        "fixture".into(),
        ScbFile {
            version: 1.0,
            classes: vec![],
        },
    );
    payload.rhs_files.insert(
        "characters/fixture.rhs".into(),
        RhsData {
            signature: 17,
            profiles: vec![],
        },
    );
    payload.sprite_bank = Some(ShippingSpriteBank {
        signature: 17,
        dictionaries: vec![],
        sprite_count: 0,
        sprites: vec![],
        vq_chunks: vec![],
        rle_jxl_chunks: vec![],
    });
    payload.raw.insert("maps/example.map".into(), vec![7, 4, 9]);
    payload
        .audio_durations_ms
        .insert("sounds/example.wav".into(), 193);
    let mission = ShippingMission::from_payload(payload);
    let frozen: FrozenMissionV8 =
        serde_json::from_value(serde_json::to_value(&mission).unwrap()).unwrap();
    let encoded = encode_mission_native(&mission);
    assert_eq!(&encoded[..8], &SHIPPING_MISSION_MAGIC);
    assert_eq!(&encoded[8..12], &8u32.to_le_bytes());
    assert_eq!(&encoded[12..], bitcode::encode(&frozen));
    let compressed = zstd_compress_with_window(&encoded, 30).unwrap();
    let decoded = decode_mission_compressed(&compressed).unwrap();
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        serde_json::to_value(&frozen).unwrap()
    );
    let prepared = decoded.prepare("fixture").unwrap();
    assert!(prepared.mission.payload.raw.is_empty());
    assert_eq!(
        prepared.mission.raw_asset("maps/example.map"),
        Some([7, 4, 9].as_slice())
    );
    assert!(
        ShippingMission::default()
            .merge_part(prepared.mission)
            .is_err()
    );
}
