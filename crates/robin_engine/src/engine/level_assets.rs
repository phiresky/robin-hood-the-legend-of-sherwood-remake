//! Focused engine level assets ownership and behavior.

// ─── Level assets (immutable after load) ────────────────────────────

/// Host opacity contract, attached through [`LevelRuntimeAttachments::pixel_opacity`].
pub use robin_content::PixelOpacityLookup;

/// Immutable level assets loaded once per mission.
///
/// These are read-only after the level-load sequence completes. They
/// never change during gameplay and are identical across every client
/// in a multiplayer session. Not serialized — the host re-attaches
/// them after deserialization from the loaded level files.
///
/// `sprite_scriptor` is a rendering asset.
/// `hiking_paths` and `profile_manager` are shared via `Arc` so cloning
/// EngineInner for rollback snapshots is a cheap reference-count bump.
///
/// Note: the former `frame_holder: Arc<robin_assets::FrameHolder>` field
/// was removed in the engine carve-out (Decision 1) so the engine crate
/// does not depend on `robin_assets`. Frame-holder-dependent operations
/// (sprite-variant dictionary setup, `signature()` / `is_pixel_opaque`)
/// now live on the host side in `robin_rs`, and the per-pixel pick
/// path reaches the packed sprite data through [`PixelOpacityLookup`].
#[derive(Clone, Default)]
pub struct LevelAssets {
    pub navigation: LevelNavigationAssets,
    pub environment: LevelEnvironmentAssets,
    pub audio: LevelAudioAssets,
    pub attachments: LevelRuntimeAttachments,
    /// Sprite script loader/cache. Loads `.rhs` animation profiles.
    /// Arc-wrapped — immutable after load, cheap to clone for rollback.
    pub sprite_scriptor: std::sync::Arc<crate::sprite_script::SpriteScriptor>,
    /// Weapon / character profiles loaded from the CPF file.
    /// Shared via `Arc` with `Campaign`.
    pub profile_manager: std::sync::Arc<crate::profiles::ProfileManager>,
    /// "Bank changed" token used by the sprite-script cache to decide
    /// whether a per-profile cache entry needs reloading. The host writes
    /// this to its frame-holder signature after the sprite bank is
    /// loaded — engine code reads it during sprite-script lookups.
    pub bank_signature: u32,
    /// Immutable mission bytecode and script-indexed authored data.
    pub scripts: LevelScriptAssets,
    /// Immutable entity identities and construction-time script attachments.
    pub entities: LevelEntityAssets,
    /// Localized peasant firstname pool (menu text IDs 100-121). Used
    /// to build civilian display names by picking a random
    /// firstname/surname for non-VIP peasants. Populated once at
    /// level-load when the text resource is attached.
    pub peasant_firstnames: Vec<String>,
    /// Localized peasant surname pool (menu text IDs 122-143).
    pub peasant_surnames: Vec<String>,
    /// Fixed VIP profile identity to localized status name (menu text
    /// IDs 144-150). Fresh VIP descriptions consume no RNG.
    pub fixed_vip_names: std::collections::BTreeMap<String, String>,
    /// Preloaded accessory-sprite prototypes, one per projectile
    /// `ObjectType` (arrow, stone, apple, net, wasp-nest, purse, coin,
    /// ale, cape). Loaded once at mission init via
    /// `EngineInner::preload_accessory_sprite_prototypes`; runtime spawn
    /// paths clone from here to hydrate `ElementData::sprite`.
    pub accessory_sprite_prototypes:
        std::collections::HashMap<crate::element::ObjectType, crate::sprite::Sprite>,
    /// Preloaded character-master sprites keyed by campaign profile. Original
    /// Player-actor initialization resolves the same profile before a
    /// dynamic PC's serialized state is read.
    pub character_sprite_prototypes:
        std::collections::HashMap<crate::profiles::CharacterProfileIdx, crate::sprite::Sprite>,
}

/// Immutable navigation geometry and exact authored topology. Runtime active bits remain in the engine.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LevelNavigationAssets {
    /// Static fast-find grid geometry built at level load. Runtime
    /// active/overlay bits live on `EngineInner::fast_grid`; snapshots
    /// reattach this Arc after decode.
    pub level_grid: std::sync::Arc<crate::fast_find_grid::LevelGrid>,
    /// Static pathfinder graph built at level load. Runtime pathfinder
    /// snapshots carry only the per-area state table; after decode the
    /// engine clones this baseline graph and reapplies those states.
    pub pathfinder_graph: std::sync::Arc<crate::pathfinder::PathGraph>,
    /// Hiking/patrol paths loaded from the mission file (PWAY/RAIL chunks).
    pub hiking_paths: std::sync::Arc<Vec<crate::level_data::RawHikingPath>>,
    /// Exact live sector identity for each `(path index, waypoint index)`.
    /// `None` is reserved for synthetic/test levels whose waypoints are
    /// intentionally number-only.
    pub hiking_waypoint_sectors:
        Option<std::sync::Arc<Vec<Vec<crate::position_interface::SectorHandle>>>>,
    /// Exact immutable construction topology of Original's
    /// original-game spatial-grid arrays. `None` is reserved for synthetic/test levels
    /// which did not retain source chunk order.
    pub legacy_grid_topology: Option<LegacyGridTopologyAssets>,
}

/// Immutable environmental geometry. Preserve authored vector order; dynamic activation lives in engine state.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LevelEnvironmentAssets {
    /// Every valid authored LIGHT/DARK polygon paired with its ambience
    /// bitmask. Runtime schedules toggle the corresponding hashed
    /// `FastFindGrid::sector_active` entries without rebuilding immutable
    /// level geometry.
    pub ambience_shadow_sectors: std::sync::Arc<Vec<(crate::fast_find_grid::SectorIndex, u32)>>,
    /// Water/hole zones for projectile-splash detection. Rebuilt from
    /// the proto material chunk at level load. Used by the water/hole
    /// determination path.
    pub water_zones: crate::water_zones::WaterZones,
    /// Full SECTOR_SOUND registry (material + polygon for every material
    /// sector) plus the map's default material. Used by the no-obstacle
    /// branch of `Engine::set_obstacle_and_material` to resolve footstep
    /// material from the actor's position. Rebuilt from
    /// `ProtoData::material_sectors` + `ProtoMisc::default_material` at
    /// level load.
    pub material_sectors: crate::material_sectors::MaterialSectors,
    /// Complete CHUNK_MATERIAL table in authored index order.
    ///
    /// LINE_SOUND edges retain indices into this table, just as Original
    /// retains a material-sector reference on each line. `None` represents a
    /// degenerate authored polygon which cannot produce boundary lines.
    pub all_material_sectors: Vec<Option<crate::material_sectors::MaterialSector>>,
    /// Static sight obstacles loaded from the level (3D occluders).
    /// Wrapped in `Arc` so cloning `LevelAssets` is a refcount bump
    /// rather than a 600+ KB deep copy. Mutated only at level load
    /// time via `Arc::make_mut`. The runtime per-obstacle active flag
    /// (toggled by `PatchEffect::SwapObjects`) lives separately on
    /// `EngineInner::static_sight_obstacle_active` — that vec
    /// participates in rollback hashing; this immutable geometry does
    /// not.
    pub static_sight_obstacles: std::sync::Arc<Vec<crate::sight_obstacle::SightObstacle>>,
}

/// Deterministic audio inputs published together before sealing a mission. Missing optional source timing remains explicit.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LevelAudioAssets {
    /// Diagnostic maximum exclamation length populated by the host at level
    /// load. Logical completion uses the concrete sound-manager resolution,
    /// never this upper bound.
    /// `Arc` so cloning `LevelAssets` is a refcount bump.
    pub(crate) exclamation_durations: ExclamationDurations,
    /// Ordered, source-authoritative speech selection metadata. Unlike
    /// `exclamation_durations` (a diagnostic maximum), this retains random
    /// gaps, exact variant order, sample identities, and each variant's
    /// duration so deterministic consumers can validate a concrete speech
    /// boundary.
    pub(crate) speech_timing_catalog: std::sync::Arc<SpeechTimingCatalog>,
    /// Exact authored, team, and reinforcement speech-profile closure
    /// selected before mission construction.
    pub required_exclamation_ids: std::collections::BTreeSet<u32>,
    /// Sample-length lookup for sound sources (sample id → sim frames).
    /// Populated by the host at level load from the decoded WAV lengths
    /// in `SoundCache::source_cache` after initializing the required
    /// source sample IDs.  The engine reads it when
    /// activating a `Single` / `Volatile` source to schedule the
    /// deterministic finish frame — so rollback replay reproduces the
    /// exact `sources.active` / `delete` transitions without depending
    /// on the audio backend's wall-clock playback-completion callback.
    pub(crate) source_durations: crate::engine::SourceDurations,
    /// Required sound-source sample IDs collected before mission preparation.
    /// The host initializes their cache and publishes timing before sealing;
    /// the engine's source-loading stage retains the same authored closure.
    pub sound_source_required_ids: std::collections::BTreeSet<u32>,
}

/// A malformed prepared table, or an explicitly unavailable ranked timing input.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error)]
pub enum AudioPreparationError {
    #[error("prepared audio duration must be positive: {0}")]
    ZeroDuration(String),
    #[error("speech group {group:#010x} variant {index} has an empty sample identity")]
    EmptySampleIdentity { group: u32, index: usize },
    #[error("ranked timing requires a nonempty speech catalog")]
    MissingSpeechCatalog,
    #[error("speech group {group:#010x} variant {index} has no source timing")]
    MissingSpeechDuration { group: u32, index: usize },
    #[error("required sound source {0:#010x} has no source timing")]
    MissingSourceDuration(u32),
}

impl LevelAudioAssets {
    /// Atomically publish all deterministic timing tables. A failed build leaves
    /// the previous tables intact. Missing samples are retained, not fabricated:
    /// ordinary/synthetic missions allow them, while ranked admission explicitly
    /// checks [`Self::validate_ranked_timing`]. This is independent of playback.
    pub fn publish_timing(
        &mut self,
        exclamations: ExclamationDurations,
        speech: std::sync::Arc<SpeechTimingCatalog>,
        sources: crate::engine::SourceDurations,
    ) -> Result<(), AudioPreparationError> {
        for (key, &duration) in exclamations.iter() {
            if duration == 0 {
                return Err(AudioPreparationError::ZeroDuration(format!(
                    "exclamation {key:?}"
                )));
            }
        }
        for (&id, &duration) in sources.iter() {
            if duration == 0 {
                return Err(AudioPreparationError::ZeroDuration(format!(
                    "source {id:#010x}"
                )));
            }
        }
        for (&group, timing) in &speech.groups {
            for (index, variant) in timing.variants.iter().enumerate() {
                if variant.sample_identity.is_empty() {
                    return Err(AudioPreparationError::EmptySampleIdentity { group, index });
                }
                if variant.duration_frames == Some(0) {
                    return Err(AudioPreparationError::ZeroDuration(format!(
                        "speech {group:#010x}/{index}"
                    )));
                }
            }
        }
        self.exclamation_durations = exclamations;
        self.speech_timing_catalog = speech;
        self.source_durations = sources;
        Ok(())
    }

    pub fn exclamation_durations(&self) -> &ExclamationDurations {
        &self.exclamation_durations
    }

    pub fn speech_timing_catalog(&self) -> &std::sync::Arc<SpeechTimingCatalog> {
        &self.speech_timing_catalog
    }

    pub fn source_durations(&self) -> &crate::engine::SourceDurations {
        &self.source_durations
    }

    /// The existing ranked completeness policy, expressed once for every
    /// driver. Empty catalogs remain valid for unranked synthetic missions.
    /// Serde decoding this data is not proof of preparation or admission.
    pub fn validate_ranked_timing(&self) -> Result<(), AudioPreparationError> {
        if self.speech_timing_catalog.groups.is_empty() {
            return Err(AudioPreparationError::MissingSpeechCatalog);
        }
        for (&group, timing) in &self.speech_timing_catalog.groups {
            for (index, variant) in timing.variants.iter().enumerate() {
                if variant.duration_frames.is_none() {
                    return Err(AudioPreparationError::MissingSpeechDuration { group, index });
                }
            }
        }
        for &id in &self.sound_source_required_ids {
            if !self.source_durations.contains_key(&id) {
                return Err(AudioPreparationError::MissingSourceDuration(id));
            }
        }
        Ok(())
    }
}

/// Process-local implementations reattached after snapshot decoding. Projection
/// includes opacity behavior and package identity, never these implementations.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LevelRuntimeAttachments {
    /// Host-provided per-pixel sprite hit-test callback. `None` before
    /// the host publishes its final loaded dictionary generation; engine code
    /// that wants per-pixel sprite pick behaviour falls back to bbox-only when
    /// missing. The concrete host publisher synchronizes later ambiance
    /// shadow-key generations across every cloned `LevelAssets` handle.
    #[serde(skip)]
    pub pixel_opacity: Option<std::sync::Arc<dyn PixelOpacityLookup>>,
    /// Process-local Spellforge VM.  Its package identity and complete event
    /// tape live in Engine state; this attachment is reconstructed at load.
    #[serde(skip)]
    pub spellforge_runtime: Option<std::sync::Arc<dyn crate::spellforge::SpellforgeRuntime>>,
}

/// Script-facing immutable level data, grouped separately from rendering and
/// navigation assets. It is populated only while constructing a mission and is
/// borrowed read-only after [`Engine`](crate::engine::Engine) creation.
#[derive(
    Clone, Debug, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct LevelScriptAssets {
    /// Pre-decoded bytecode in host load order, keyed by mission base name.
    pub mission_programs: std::sync::Arc<
        std::collections::BTreeMap<String, std::sync::Arc<crate::script_manager::ScriptProgram>>,
    >,
    /// Exact mission script identity selected during construction.
    pub mission_name: Option<String>,
    /// Spellforge Lua name tables. Vanilla missions leave these empty.
    pub names: std::sync::Arc<crate::natives::ScriptNameBindings>,
    /// Number of authored script locations.
    pub location_count: usize,
    /// Number of point locations at the front of the location arrays.
    pub point_count: usize,
    /// Positions of points, lines, then sectors in authored order.
    pub location_positions: std::sync::Arc<Vec<(f32, f32)>>,
    /// Layers parallel to `location_positions`.
    pub location_layers: std::sync::Arc<Vec<u16>>,
    /// Motion-sector numbers parallel to `location_positions`.
    pub location_sectors: std::sync::Arc<Vec<u16>>,
    /// Exact Original position-sector identities parallel to
    /// `location_positions`. Entries remain `None` only for legacy/test
    /// bindings that predate retained sparse-sector topology.
    #[serde(default)]
    pub location_sector_handles:
        std::sync::Arc<Vec<Option<crate::position_interface::SectorHandle>>>,
    /// Number of buildings exposed to the mission script.
    pub building_count: usize,
    /// Number of hiking paths exposed to the mission script.
    pub hiking_path_count: usize,
    /// Fast-grid indices for authored script zones, in authored sector order.
    pub zone_grid_indices: std::sync::Arc<Vec<u32>>,
}

/// Immutable entity bindings created while loading a mission.
#[derive(
    Clone, Debug, Default, serde::Serialize, serde::Deserialize, bitcode::Encode, bitcode::Decode,
)]
pub struct LevelEntityAssets {
    /// Number of authored mobile elements required by compatible snapshots.
    pub mobile_element_count: usize,
    /// Patch index to optional FX actor handle, in proto-then-mission order.
    pub patch_animation_entities: std::sync::Arc<Vec<Option<i32>>>,
    /// Scroll entity IDs in authored creation order.
    pub scroll_entity_ids: Vec<crate::engine::EntityId>,
    /// Soldier load-order index to typed entity ID.
    pub soldier_entity_ids: Vec<crate::engine::EntityId>,
    /// Soldier load-order index to subordinate soldier load-order IDs.
    pub soldier_subordinate_ids: Vec<Vec<u16>>,
    /// Exact data order needed to reconstruct original-game element creation
    /// orders after the parsed level data has been released.
    pub legacy_proto_element_chunk_order: Vec<crate::level_data::ProtoElementChunk>,
    pub legacy_mission_element_chunk_order: Vec<crate::level_data::MissionElementChunk>,
    pub legacy_mission_element_group_order: Vec<crate::level_data::MissionElementGroup>,
    pub legacy_proto_patch_count: usize,
    pub legacy_proto_animation_count: usize,
}

/// Data-derived identity for a patch in the original game's per-layer serialization
/// walk.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct LegacyGridPatchAsset {
    pub patch_index: u32,
    pub layer: u16,
    pub index_in_layer: u16,
    /// Rust handle of the patch-owned FX. Original always constructs this
    /// object, even when its frame-profile name is empty.
    pub fx_entity_handle: Option<i32>,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum LegacyGridGateAsset {
    Door,
    Stateless,
}

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum LegacyGridScriptObjectAsset {
    NonSector,
    Sector { associated_class: Option<String> },
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
pub enum LegacyGridSectorAsset {
    NullOrOrdinary,
    Door { gate_index: u32 },
    Building,
    Lift,
}

/// Exact ordered arrays traversed by original-game spatial-grid saves.
#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct LegacyGridTopologyAssets {
    pub patches: Vec<LegacyGridPatchAsset>,
    pub gates: Vec<LegacyGridGateAsset>,
    /// Runtime jump-line order mapped to the original game's layer and within-layer index
    /// identities. The original game serializes these through the combined line
    /// array, which also contains motion boundaries and elevation lines.
    pub jump_line_identities: Vec<(u16, i16)>,
    pub script_objects: Vec<LegacyGridScriptObjectAsset>,
    pub sectors: Vec<LegacyGridSectorAsset>,
    /// Maps original-game sparse sector-reference slots to
    /// Rust's compact runtime motion/building sector-number domain. Slots for
    /// non-position sectors and constructor holes remain `None`.
    pub position_sector_numbers: Vec<Option<i16>>,
    /// Maps the same sparse original-game sector slots to exact entries in
    /// the fast-grid sector array. Unlike `position_sector_numbers`, this
    /// retains pointer identity when multiple runtime polygons expose the
    /// same public sector number.
    #[serde(default)]
    pub position_sector_indices: Vec<Option<crate::fast_find_grid::SectorIndex>>,
    /// Sparse original-game sector construction number for each proto jump
    /// zone, in the same order as the runtime jump-sector arena entries.
    #[serde(default)]
    pub jump_sector_numbers: Vec<u16>,
}

/// Sample duration in sim frames (40 ms each), keyed by
/// `(group, profile_id, exclamation_id)`. Lives on `EngineInner` (so it
/// rides along in rollback snapshots cheaply via `Arc`); the host
/// populates it at level load by walking the sound cache. EngineInner
/// reads this when an NPC speaks to schedule the deterministic
/// MYTALK finish — instead of waiting for the audio backend's
/// wall-clock playback completion, which doesn't replay during
/// rollback. As in the original sound hourglass, a missing sample has
/// length zero and completes at the next scheduling boundary.
pub type ExclamationDurations =
    std::sync::Arc<std::collections::BTreeMap<(crate::sound::ExclamationGroup, u32, u16), u32>>;

/// Exact ordered speech-selection metadata used by deterministic audio
/// boundaries. Group keys match `SoundCache::speech_cache.groups`.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SpeechTimingCatalog {
    pub groups: std::collections::BTreeMap<u32, SpeechTimingGroup>,
}

impl SpeechTimingCatalog {
    /// Whether every listed variant has source-authoritative timing.
    pub fn is_complete(&self) -> bool {
        self.groups
            .values()
            .flat_map(|group| &group.variants)
            .all(|variant| variant.duration_frames.is_some())
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SpeechTimingGroup {
    /// Original-game sound-group gaps: the number of random misses appended
    /// to this group's ordered sample variants.
    pub gaps: u16,
    /// Exact source order. Selection indexes this vector before its sample
    /// identity and duration are used, so it deliberately remains a `Vec`.
    pub variants: Vec<SpeechTimingVariant>,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct SpeechTimingVariant {
    pub sample_identity: String,
    /// `None` explicitly records missing source timing. Metadata construction
    /// preserves that state; callers that require complete timing must reject
    /// it instead of inventing a duration.
    pub duration_frames: Option<u32>,
}

#[cfg(test)]
mod speech_timing_metadata_tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    use super::{
        AudioPreparationError, LevelAssets, LevelAudioAssets, LevelRuntimeAttachments,
        SpeechTimingCatalog, SpeechTimingGroup, SpeechTimingVariant,
    };

    fn prepared_audio(duration: Option<u32>) -> LevelAudioAssets {
        let mut audio = LevelAudioAssets::default();
        audio.required_exclamation_ids.insert(0x1000_0000);
        audio.sound_source_required_ids.insert(7);
        audio
            .publish_timing(
                Arc::new(BTreeMap::new()),
                Arc::new(SpeechTimingCatalog {
                    groups: BTreeMap::from([(0x1000_0001, group(2, "first.wav", duration))]),
                }),
                Arc::new(BTreeMap::from([(7, 11)])),
            )
            .unwrap();
        audio
    }

    #[test]
    fn prepared_audio_publication_preserves_order_and_missing_optional_timing() {
        let audio = prepared_audio(None);
        assert_eq!(audio.speech_timing_catalog().groups[&0x1000_0001].gaps, 2);
        assert_eq!(
            audio.speech_timing_catalog().groups[&0x1000_0001].variants[0].duration_frames,
            None
        );
        assert_eq!(
            audio.validate_ranked_timing(),
            Err(AudioPreparationError::MissingSpeechDuration {
                group: 0x1000_0001,
                index: 0,
            })
        );
        assert!(prepared_audio(Some(11)).validate_ranked_timing().is_ok());
    }

    #[test]
    fn prepared_audio_required_source_completeness_is_explicit() {
        let mut audio = prepared_audio(Some(11));
        audio.sound_source_required_ids.insert(8);
        assert_eq!(
            audio.validate_ranked_timing(),
            Err(AudioPreparationError::MissingSourceDuration(8))
        );
        assert!(!audio.source_durations().contains_key(&8));
    }

    #[test]
    fn prepared_audio_failed_publication_preserves_all_old_tables() {
        let mut audio = prepared_audio(Some(11));
        let old = audio.clone();
        let error = audio
            .publish_timing(
                Arc::new(BTreeMap::new()),
                Arc::new(SpeechTimingCatalog::default()),
                Arc::new(BTreeMap::from([(7, 0)])),
            )
            .unwrap_err();
        assert!(matches!(error, AudioPreparationError::ZeroDuration(_)));
        assert!(Arc::ptr_eq(
            audio.source_durations(),
            old.source_durations()
        ));
        assert!(Arc::ptr_eq(
            audio.speech_timing_catalog(),
            old.speech_timing_catalog()
        ));
        assert!(Arc::ptr_eq(
            audio.exclamation_durations(),
            old.exclamation_durations()
        ));
    }

    #[test]
    fn prepared_audio_rejects_malformed_concrete_speech_without_erasing_missing_samples() {
        let mut audio = LevelAudioAssets::default();
        for (sample, duration, expected_empty) in [("", None, true), ("voice.wav", Some(0), false)]
        {
            let result = audio.publish_timing(
                Arc::new(BTreeMap::new()),
                Arc::new(SpeechTimingCatalog {
                    groups: BTreeMap::from([(1, group(0, sample, duration))]),
                }),
                Arc::new(BTreeMap::new()),
            );
            if expected_empty {
                assert_eq!(
                    result,
                    Err(AudioPreparationError::EmptySampleIdentity { group: 1, index: 0 })
                );
            } else {
                assert!(matches!(
                    result,
                    Err(AudioPreparationError::ZeroDuration(_))
                ));
            }
        }
    }

    #[test]
    fn prepared_audio_synthetic_empty_and_deserialized_missing_inputs_are_not_ranked_proofs() {
        let mut empty = LevelAudioAssets::default();
        empty
            .publish_timing(Arc::default(), Arc::default(), Arc::default())
            .unwrap();
        assert_eq!(
            empty.validate_ranked_timing(),
            Err(AudioPreparationError::MissingSpeechCatalog)
        );
        let restored: LevelAudioAssets =
            serde_json::from_str(&serde_json::to_string(&prepared_audio(None)).unwrap()).unwrap();
        assert_eq!(
            restored.validate_ranked_timing(),
            prepared_audio(None).validate_ranked_timing()
        );
    }

    #[test]
    fn prepared_asset_groups_retain_snapshot_sharing_and_attachment_absence() {
        let mut original = LevelAssets::new();
        original.audio = prepared_audio(Some(11));
        let snapshot = original.clone();
        assert!(Arc::ptr_eq(
            &snapshot.navigation.level_grid,
            &original.navigation.level_grid
        ));
        assert!(Arc::ptr_eq(
            &snapshot.environment.static_sight_obstacles,
            &original.environment.static_sight_obstacles
        ));
        assert!(Arc::ptr_eq(
            snapshot.audio.speech_timing_catalog(),
            original.audio.speech_timing_catalog()
        ));
        let restored: LevelRuntimeAttachments = serde_json::from_str("{}").unwrap();
        assert!(restored.pixel_opacity.is_none());
        assert!(restored.spellforge_runtime.is_none());
    }

    fn group(gaps: u16, sample: &str, duration_frames: Option<u32>) -> SpeechTimingGroup {
        SpeechTimingGroup {
            gaps,
            variants: vec![SpeechTimingVariant {
                sample_identity: sample.to_owned(),
                duration_frames,
            }],
        }
    }

    #[test]
    fn equal_metadata_has_identical_canonical_binary_regardless_of_insert_order() {
        let mut ascending = BTreeMap::new();
        ascending.insert(0x1000_0001, group(2, "first.wav", Some(11)));
        ascending.insert(0x2000_0002, group(3, "second.wav", Some(19)));

        let mut descending = BTreeMap::new();
        descending.insert(0x2000_0002, group(3, "second.wav", Some(19)));
        descending.insert(0x1000_0001, group(2, "first.wav", Some(11)));

        let left = (
            SpeechTimingCatalog { groups: ascending },
            BTreeSet::from([0x2000_0000, 0x1000_0000]),
        );
        let right = (
            SpeechTimingCatalog { groups: descending },
            BTreeSet::from([0x1000_0000, 0x2000_0000]),
        );

        assert_eq!(left, right);
        assert_eq!(bitcode::encode(&left), bitcode::encode(&right));
        assert_eq!(
            serde_json::to_vec(&left).expect("serialize speech timing metadata"),
            serde_json::to_vec(&right).expect("serialize speech timing metadata")
        );
    }

    #[test]
    fn source_variant_order_is_preserved_by_the_metadata_seal() {
        let forward = SpeechTimingCatalog {
            groups: BTreeMap::from([(
                7,
                SpeechTimingGroup {
                    gaps: 1,
                    variants: vec![
                        SpeechTimingVariant {
                            sample_identity: "a.wav".into(),
                            duration_frames: Some(4),
                        },
                        SpeechTimingVariant {
                            sample_identity: "b.wav".into(),
                            duration_frames: Some(6),
                        },
                    ],
                },
            )]),
        };
        let mut reversed = forward.clone();
        reversed.groups.get_mut(&7).unwrap().variants.reverse();

        assert_ne!(forward, reversed);
        assert_ne!(bitcode::encode(&forward), bitcode::encode(&reversed));
    }

    #[test]
    fn incomplete_source_timing_is_retained_and_reported() {
        let complete = SpeechTimingCatalog {
            groups: BTreeMap::from([(1, group(0, "present.wav", Some(5)))]),
        };
        let incomplete = SpeechTimingCatalog {
            groups: BTreeMap::from([(1, group(0, "missing.wav", None))]),
        };

        assert!(complete.is_complete());
        assert!(!incomplete.is_complete());
        let decoded: SpeechTimingCatalog = bitcode::decode(&bitcode::encode(&incomplete))
            .expect("decode incomplete timing metadata");
        assert_eq!(decoded, incomplete);
        assert!(!decoded.is_complete());
    }
}

impl LevelNavigationAssets {
    pub(crate) fn hiking_waypoint_sector(
        &self,
        path_index: usize,
        waypoint_index: usize,
        public_sector: u16,
    ) -> Option<crate::position_interface::SectorHandle> {
        let Some(paths) = &self.hiking_waypoint_sectors else {
            return crate::position_interface::SectorHandle::new(public_sector);
        };
        let exact = paths
            .get(path_index)
            .and_then(|path| path.get(waypoint_index))
            .copied()
            .unwrap_or_else(|| {
                panic!(
                    "required exact hiking waypoint identity is missing for path {path_index} waypoint {waypoint_index}"
                )
            });
        assert_eq!(
            exact.get(),
            public_sector,
            "hiking waypoint path {path_index} waypoint {waypoint_index} public/exact identity conflict"
        );
        Some(exact)
    }
}

impl LevelAssets {
    /// Mutable access to sprite_scriptor during initialization.
    pub fn sprite_scriptor_mut(&mut self) -> &mut crate::sprite_script::SpriteScriptor {
        std::sync::Arc::make_mut(&mut self.sprite_scriptor)
    }

    pub fn new() -> Self {
        Self {
            navigation: LevelNavigationAssets::default(),
            environment: LevelEnvironmentAssets::default(),
            audio: LevelAudioAssets::default(),
            attachments: LevelRuntimeAttachments::default(),
            sprite_scriptor: std::sync::Arc::new(crate::sprite_script::SpriteScriptor::new()),
            profile_manager: std::sync::Arc::new(crate::profiles::ProfileManager::new()),
            bank_signature: 0,
            scripts: LevelScriptAssets::default(),
            entities: LevelEntityAssets::default(),
            peasant_firstnames: Vec::new(),
            peasant_surnames: Vec::new(),
            fixed_vip_names: std::collections::BTreeMap::new(),
            accessory_sprite_prototypes: std::collections::HashMap::new(),
            character_sprite_prototypes: std::collections::HashMap::new(),
        }
    }

    /// Pick a deterministic firstname+surname for a civilian using
    /// `seed` as the index. Returns `None` if the name pool hasn't
    /// been populated.
    pub fn random_peasant_name(&self, seed: usize) -> Option<String> {
        if self.peasant_firstnames.is_empty() || self.peasant_surnames.is_empty() {
            return None;
        }
        let f = &self.peasant_firstnames[seed % self.peasant_firstnames.len()];
        let l = &self.peasant_surnames[(seed / self.peasant_firstnames.len().max(1) + seed * 7)
            % self.peasant_surnames.len()];
        Some(format!("{f} {l}"))
    }
}
