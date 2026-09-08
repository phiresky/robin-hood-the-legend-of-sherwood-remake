//! Frozen pre-refactor v15 descriptor. Do not update when rearranging runtime
//! representations; a deliberate format change must introduce a new version.
use super::*;
#[derive(Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct FrozenShippingV15 {
    pub profiles: Option<ProfileManager>,
    pub res_files: std::collections::BTreeMap<String, ResourceManager>,
    pub pak_files: std::collections::BTreeMap<String, Vec<EncodedPicture>>,
    pub red_files: std::collections::BTreeMap<String, LevelDescriptors>,
    /// Keyed by mission base name (no extension), e.g. `"Dem_Lei_MP"`.
    pub levels: std::collections::BTreeMap<String, LoadedLevel>,
    pub scripts: std::collections::BTreeMap<String, ScbFile>,
    /// Keyed by the full relative path `Characters/<name>.rhs`.
    pub rhs_files: std::collections::BTreeMap<String, RhsData>,
    /// Packed sprite pool. See [`ShippingSpriteBank`].
    pub sprite_bank: Option<ShippingSpriteBank>,
    /// Terrain bitmaps and other not-yet-parsed binary blobs, keyed by
    /// relative path (e.g. `Levels/Day/leicester.map`).
    pub raw: std::collections::BTreeMap<String, Vec<u8>>,
    /// Source-authoritative durations for boot audio stored in `raw`.
    pub audio_durations_ms: BTreeMap<String, u32>,
    /// Standalone browser audio, keyed by the normalized logical Opus path
    /// (for example `sounds/snd_001.opus`). The encoded bytes intentionally do
    /// not live in this bitcode manifest or any mission payload.
    pub audio_assets: BTreeMap<String, ShippingAudioAsset>,
    /// Independently compressed payload to fetch before starting each mission.
    pub missions: BTreeMap<String, ShippingMissionRef>,
    /// Content-addressed RHS payloads required when a character profile can
    /// participate in the selected mission. Keys are stable CPF character
    /// profile indices; values include that exact physical character RHS and
    /// the object/projectile RHS files enabled by its actions.
    pub character_rhs_files: BTreeMap<u32, Vec<String>>,
    /// Content-addressed localized voice payloads for each CPF character
    /// profile. Runtime party/reinforcement selection uses the same profile
    /// closure as `character_rhs_files`, avoiding every PC voice in every
    /// mission reference.
    pub character_audio_files: BTreeMap<u32, Vec<String>>,
    /// Exclamation profile id corresponding to each CPF character profile.
    pub character_exclamation_ids: BTreeMap<u32, u32>,
    /// Authored soldier/civilian/required/rescue exclamation ids for each
    /// mission. Dynamic party ids are unioned at the mission-load boundary.
    pub mission_exclamation_ids: BTreeMap<String, Vec<u32>>,
    /// Conservative RHS closure used only when constructing a mission around
    /// an already-decoded saved world. Saved entities may contain object types
    /// that are neither authored by the destination mission nor implied by its
    /// current party, so save launches must not silently omit their masters.
    pub saved_world_rhs_files: Vec<String>,
    /// Language packs keyed by canonical BCP-47 locale (`"en-US"`,
    /// `"de-DE"`, ...). Windows' invariant LCID 2047 is represented as
    /// `"und"`; its legacy `"2047"` and `"neutral"` names remain accepted
    /// aliases but are not promoted to a made-up language identity.
    #[serde(default)]
    pub locales: BTreeMap<String, ShippingLocale>,
}

#[test]
fn v15_wire_and_json_match_frozen_descriptor_with_runtime_state() {
    let mut datadir = ShippingDatadir::default();
    datadir.profiles = Some(ProfileManager::default());
    datadir
        .res_files
        .insert("fixture.res".into(), ResourceManager::new());
    let installation = datadir.installation_id();
    assert_eq!(installation, datadir.installation_id());
    assert_ne!(installation, ShippingDatadir::default().installation_id());
    datadir
        .raw
        .insert("text/fixture.dat".into(), vec![1, 2, 3, 4]);
    datadir
        .audio_durations_ms
        .insert("sounds/example.wav".into(), 137);
    datadir.character_exclamation_ids.insert(7, 19);
    datadir
        .mission_exclamation_ids
        .insert("fixture".into(), vec![1, 2]);
    datadir.saved_world_rhs_files.push("rhs/fixture.bin".into());
    datadir
        .locales
        .insert("en-US".into(), ShippingLocale::default());
    let frozen: FrozenShippingV15 =
        serde_json::from_value(serde_json::to_value(&datadir).unwrap()).unwrap();
    let expected = bitcode::encode(&frozen);
    let before = encode_native(&datadir);
    assert_eq!(&before[..8], b"RHDDNA15");
    assert_eq!(&before[8..12], &15u32.to_le_bytes());
    assert_eq!(&before[12..], expected);
    datadir.set_remote_base_url("https://invalid.example/assets".into());
    datadir.runtime.source_dir = Some(PathBuf::from("/example"));
    datadir
        .runtime
        .preloaded_files
        .write()
        .unwrap()
        .insert("staged".into(), Arc::new(vec![9]));
    assert_eq!(encode_native(&datadir), before);
    let decoded = decode_native(&before).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        serde_json::to_value(frozen).unwrap()
    );
}
