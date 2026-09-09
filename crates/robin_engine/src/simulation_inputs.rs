//! Canonical deterministic mission-input projection.
//!
//! Loose legacy datadirs and selective shipping payloads intentionally use
//! different physical representations. Ranked replay identity therefore
//! cannot hash files or encoded media: it has to bind the parsed values which
//! can affect simulation. This module is that shared semantic boundary.
//!
//! Every floating-point value is represented by its IEEE-754 bits. Sprite and
//! background pixels and encoded audio are deliberately absent, while sprite
//! geometry/animation metadata, map geometry, and source-authoritative audio
//! durations are included.

use std::collections::BTreeMap;

use robin_run_protocol::{
    CanonicalDocument, ContentClosureKindV1, ContentManifestV1, Digest32, OfficialContentEditionV1,
    OfficialContentSubjectV1, PreparedMissionInputsSealV1, RulesConfigIdentityV1, SimulationSeed64,
    SpeechTimingAuthorityV1, Validate, canonical_json_bytes,
};
pub use robin_run_protocol::{
    CanonicalValue as CanonicalSimulationValue, SimulationContentComponentDocumentV1,
    SimulationContentComponentKindV1,
};
use serde::{Deserialize, Serialize};

use crate::campaign::Campaign;
use crate::engine::{Engine, GroundMarkSpriteData, LevelAssets, RankedSimulationPolicy, SimConfig};
use crate::level_data::LoadedLevel;

pub const SIMULATION_CONTENT_DOCUMENT_SCHEMA_V1: u32 = 1;
pub const SIMULATION_CONTENT_COMPONENT_SCHEMA_V1: u32 = 1;
pub const PREPARED_MISSION_RUN_PROJECTION_SCHEMA_V1: u32 = 2;
const _: () = assert!(crate::replay::REPLAY_SCHEMA_VERSION == 32);

/// Decode the exact canonical official projection SimConfig and prove that no
/// missing or unknown field was normalized away. Operator tooling and the
/// exporter share this gate so local defaults can never fill a receipt claim.
pub fn validate_official_projection_rules_config_v1(
    rules_config: &RulesConfigIdentityV1,
) -> Result<SimConfig, ProjectionError> {
    let (sim_config, _) = validate_ranked_simulation_policy_rules_config_v1(rules_config)?;
    Ok(sim_config)
}

/// Decode and seal the exact typed policy in a current ranked rules
/// configuration. Missing/unknown `SimConfig` fields, noncanonical defaults,
/// unsupported policy versions, and preset/config disagreement all fail.
pub fn validate_ranked_simulation_policy_rules_config_v1(
    rules_config: &RulesConfigIdentityV1,
) -> Result<(SimConfig, RankedSimulationPolicy), ProjectionError> {
    rules_config.validate()?;
    if rules_config.replay_schema_version != crate::replay::REPLAY_SCHEMA_VERSION {
        return Err(ProjectionError::InvalidRankedSimulationPolicy(format!(
            "ranked simulation policy V1 requires replay schema {}",
            crate::replay::REPLAY_SCHEMA_VERSION
        )));
    }
    let input = CanonicalSimulationValue::Object(rules_config.sim_config.clone());
    let sim_config: SimConfig = serde_json::from_value(
        serde_json::to_value(&input)
            .map_err(|error| ProjectionError::InvalidRankedSimulationPolicy(error.to_string()))?,
    )
    .map_err(|error| ProjectionError::InvalidRankedSimulationPolicy(error.to_string()))?;
    sim_config
        .validate()
        .map_err(|error| ProjectionError::InvalidRankedSimulationPolicy(error.to_string()))?;
    // CanonicalValue intentionally has signed and unsigned in-memory integer
    // variants, but canonical JSON has one integral number representation.
    // Operator-authored JSON therefore cannot preserve Rust's source integer
    // type. Compare the exact canonical JSON forms so a valid authored u16 is
    // accepted without permitting missing, unknown, defaulted, or substituted
    // configuration fields.
    if canonical_json_bytes(&sim_config)? != canonical_json_bytes(&input)? {
        return Err(ProjectionError::InvalidRankedSimulationPolicy(
            "canonical SimConfig has missing, unknown, or noncanonical fields".into(),
        ));
    }
    let policy =
        RankedSimulationPolicy::from_config(rules_config.ranked_simulation_policy, sim_config)
            .map_err(|error| ProjectionError::InvalidRankedSimulationPolicy(error.to_string()))?;
    policy
        .validate_config(sim_config)
        .map_err(|error| ProjectionError::InvalidRankedSimulationPolicy(error.to_string()))?;
    Ok((sim_config, policy))
}

/// Capture every supported gameplay setting while preserving the published
/// ranking metadata. Validation seals this exact configuration for playback.
pub fn custom_rules_config_v1(
    baseline: &RulesConfigIdentityV1,
    config: SimConfig,
) -> Result<RulesConfigIdentityV1, ProjectionError> {
    use crate::player_profile::DifficultyLevel;
    use robin_run_protocol::{
        RankedSimulationDifficultyV1 as Difficulty, RankedSimulationPresetV1,
    };
    let mut rules = baseline.clone();
    rules.ranked_simulation_policy.preset = RankedSimulationPresetV1::Custom;
    rules.ranked_simulation_policy.difficulty = match config.difficulty {
        DifficultyLevel::Easy => Difficulty::Easy,
        DifficultyLevel::Medium => Difficulty::Medium,
        DifficultyLevel::Hard => Difficulty::Hard,
        DifficultyLevel::Legendary => Difficulty::Legendary,
        DifficultyLevel::Custom(_) => Difficulty::Custom,
    };
    rules.sim_config = serde_json::from_value(
        serde_json::to_value(config)
            .map_err(|error| ProjectionError::InvalidRankedSimulationPolicy(error.to_string()))?,
    )
    .map_err(|error| ProjectionError::InvalidRankedSimulationPolicy(error.to_string()))?;
    validate_ranked_simulation_policy_rules_config_v1(&rules)?;
    Ok(rules)
}

/// Reconstruct the fresh campaign from the admitted profile catalog and exact
/// difficulty. A run's proposed starting save never supplies this authority.
pub fn canonical_fresh_campaign_artifact_v1(
    rules: &RulesConfigIdentityV1,
    profiles_document: &SimulationContentComponentDocumentV1,
) -> Result<robin_run_protocol::ArtifactRefV1, ProjectionError> {
    let profiles = profile_manager_from_component_document_v1(profiles_document)?;
    if profiles.characters.len() < 2 || profiles.missions.is_empty() {
        return Err(ProjectionError::InvalidRankedSimulationPolicy(
            "official profiles cannot construct a fresh campaign".into(),
        ));
    }
    let (config, _) = validate_ranked_simulation_policy_rules_config_v1(rules)?;
    let campaign = Campaign::from_profiles(&profiles, config.difficulty);
    campaign
        .validate_history_schema()
        .map_err(ProjectionError::InvalidRankedSimulationPolicy)?;
    let bytes = bitcode::encode(&campaign);
    if bytes.is_empty() || bytes.len() > 64 * 1024 * 1024 {
        return Err(ProjectionError::InvalidRankedSimulationPolicy(
            "fresh campaign exceeds verifier artifact limit".into(),
        ));
    }
    Ok(robin_run_protocol::ArtifactRefV1 {
        sha256: Digest32::digest_bytes(&bytes),
        byte_length: bytes.len() as u64,
        media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
    })
}

/// Check the real pre-Engine campaign checkpoint, including the official team
/// and mission selection. The operator's unselected fresh template is not a
/// byte-for-byte representation of a game after these setup transitions.
pub fn validate_canonical_mission_start_v1(
    rules: &RulesConfigIdentityV1,
    profiles_document: &SimulationContentComponentDocumentV1,
    edition: OfficialContentEditionV1,
    subject: &OfficialContentSubjectV1,
    simulation_seed: u64,
    expected: &robin_run_protocol::ArtifactRefV1,
    files: &crate::sbfile::SbFileSystem,
) -> Result<(), ProjectionError> {
    let profiles = profile_manager_from_component_document_v1(profiles_document)?;
    let (config, _) = validate_ranked_simulation_policy_rules_config_v1(rules)?;
    if profiles.characters.len() < 2 || profiles.missions.is_empty() {
        return Err(ProjectionError::InvalidRankedSimulationPolicy(
            "official profiles cannot construct mission setup".into(),
        ));
    }
    let mission_index = profiles
        .missions
        .iter()
        .position(|mission| mission.mission_filename == subject.mission_id())
        .ok_or_else(|| {
            ProjectionError::InvalidRankedSimulationPolicy(
                "official starting mission is absent from profiles".into(),
            )
        })?;
    let matches = |campaign: &Campaign| {
        let bytes = bitcode::encode(campaign);
        expected.byte_length == bytes.len() as u64
            && expected.sha256 == Digest32::digest_bytes(bytes)
    };
    let mut fresh = Campaign::from_profiles(&profiles, config.difficulty);
    fresh.reset(&profiles, config.difficulty);
    // Direct mission launch is also a legitimate fresh start. Its pending
    // mission selection and preselected restart checkpoint are recorded.
    let mut direct = fresh.clone();
    direct.force_next_mission(mission_index);
    direct.current_mission_idx = Some(mission_index);
    direct.snapshot_preselected_with_simulation(simulation_seed, config);
    if matches(&direct) {
        return Ok(());
    }
    match edition {
        OfficialContentEditionV1::Demo => {
            let team = match subject.mission_id() {
                "Dem_Lei_MP" => "RJMTF",
                "Demo_Lin" => "RSABC",
                _ => {
                    return Err(ProjectionError::InvalidRankedSimulationPolicy(
                        "unknown official demo starting team".into(),
                    ));
                }
            };
            let existing_files = profiles
                .characters
                .iter()
                .map(|profile| {
                    let path = format!("Data/Characters/{}.rhs", profile.filename);
                    files
                        .try_exists(&path)
                        .map(|exists| (path, exists))
                        .map_err(|status| {
                            ProjectionError::InvalidRankedSimulationPolicy(format!(
                                "cannot resolve official character file: {status}"
                            ))
                        })
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            fresh.create_gang_from_pcs_with_file_exists(
                team,
                &profiles,
                config.difficulty,
                |path| {
                    *existing_files
                        .get(path)
                        .expect("every profile file was checked")
                },
            );
            fresh.add_all_to_mission_team();
            fresh.current_mission_idx = Some(mission_index);
            fresh.snapshot_preselected_with_simulation(simulation_seed, config);
            if matches(&fresh) {
                return Ok(());
            }
        }
        OfficialContentEditionV1::Full => {
            if subject.mission_id()
                == robin_run_protocol::OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1
            {
                // A new campaign begins with the application-owned seed zero.
                // Mission selection may advance it; both checkpoints must agree.
                fresh.snapshot_with_simulation(0, config);
                let (selected, selected_index, selected_seed, selected_config) =
                    Engine::select_next_mission(fresh, &profiles, 0, config);
                if selected_index == mission_index
                    && selected_seed == simulation_seed
                    && selected_config == config
                    && matches(&selected)
                {
                    return Ok(());
                }
            }
        }
    }
    Err(ProjectionError::InvalidRankedSimulationPolicy(
        "starting campaign differs from official fresh mission setup".into(),
    ))
}

pub const SIMULATION_CONTENT_COMPONENT_ORDER_V1: [SimulationContentComponentKindV1; 8] = [
    SimulationContentComponentKindV1::Profiles,
    SimulationContentComponentKindV1::LoadedLevel,
    SimulationContentComponentKindV1::MissionScripts,
    SimulationContentComponentKindV1::SpriteSimulationMetadata,
    SimulationContentComponentKindV1::MapGeometryMetadata,
    SimulationContentComponentKindV1::LocalizedDeterministicText,
    SimulationContentComponentKindV1::SoundDurationTables,
    SimulationContentComponentKindV1::InterfaceSimulationMetadata,
];

fn canonical_from_serializable(
    value: &(impl Serialize + ?Sized),
) -> Result<CanonicalSimulationValue, ProjectionError> {
    canonicalize_serde_value(serde_value::to_value(value)?)
}

fn component_document(
    kind: SimulationContentComponentKindV1,
    payload: impl Serialize,
) -> Result<SimulationContentComponentDocumentV1, ProjectionError> {
    Ok(SimulationContentComponentDocumentV1 {
        schema_version: SIMULATION_CONTENT_DOCUMENT_SCHEMA_V1,
        kind,
        component_schema_version: SIMULATION_CONTENT_COMPONENT_SCHEMA_V1,
        payload: canonical_from_serializable(&payload)?,
    })
}

#[derive(Serialize)]
struct ProfilesPayload<'a> {
    profile_manager: &'a crate::profiles::ProfileManager,
}

/// Project the exact profile catalog through the same canonical boundary used
/// by every official mission. Operator campaign-template authoring uses this
/// to prove that the decoded shipping profiles are the profiles admitted by
/// the content authority before deriving fresh campaign bytes.
pub fn profiles_component_document_v1(
    profile_manager: &crate::profiles::ProfileManager,
) -> Result<SimulationContentComponentDocumentV1, ProjectionError> {
    component_document(
        SimulationContentComponentKindV1::Profiles,
        ProfilesPayload { profile_manager },
    )
}

/// Recover the exact typed profile catalog carried by a canonical Profiles
/// component.
///
/// Projection deliberately represents floats by their IEEE bit patterns, so
/// ordinary JSON deserialization is not the inverse of
/// [`profiles_component_document_v1`]. This decoder reverses that canonical
/// representation, deserializes the current typed [`ProfileManager`], and
/// then requires an exact projection round trip. The round trip makes this a
/// schema-bound inverse rather than a permissive JSON import surface.
pub fn profile_manager_from_component_document_v1(
    document: &SimulationContentComponentDocumentV1,
) -> Result<crate::profiles::ProfileManager, ProjectionError> {
    document.validate()?;
    if document.kind != SimulationContentComponentKindV1::Profiles
        || document.schema_version != SIMULATION_CONTENT_DOCUMENT_SCHEMA_V1
        || document.component_schema_version != SIMULATION_CONTENT_COMPONENT_SCHEMA_V1
    {
        return Err(ProjectionError::InvalidProfilesComponent(
            "document does not name the current Profiles component schema".to_owned(),
        ));
    }

    let CanonicalSimulationValue::Object(payload) = &document.payload else {
        return Err(ProjectionError::InvalidProfilesComponent(
            "Profiles payload is not an object".to_owned(),
        ));
    };
    if payload.len() != 1 || !payload.contains_key("profile_manager") {
        return Err(ProjectionError::InvalidProfilesComponent(
            "Profiles payload must contain exactly profile_manager".to_owned(),
        ));
    }
    let encoded = canonical_to_serde_value(
        payload
            .get("profile_manager")
            .expect("profile_manager key shape checked above"),
    )?;
    let profiles: crate::profiles::ProfileManager = encoded.deserialize_into()?;
    // Native projection bytes normalize nonnegative integers to unsigned.
    // Compare canonical bytes because the source Rust signedness may differ
    // after decoding while the exact semantic identity stays unchanged.
    if profiles_component_document_v1(&profiles)?.bitcode_bytes()? != document.bitcode_bytes()? {
        return Err(ProjectionError::InvalidProfilesComponent(
            "typed ProfileManager does not re-project to the exact admitted component".to_owned(),
        ));
    }
    Ok(profiles)
}

/// A document plus the exact canonical bytes and digest exposed to operator
/// tooling. Keeping the bytes alongside the typed document makes comparison
/// diagnostics possible without another serialization pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedSimulationContentComponentV1 {
    pub document: SimulationContentComponentDocumentV1,
    pub canonical_bytes: Vec<u8>,
    pub sha256: Digest32,
}

impl ProjectedSimulationContentComponentV1 {
    fn from_document(
        document: SimulationContentComponentDocumentV1,
    ) -> Result<Self, ProjectionError> {
        let canonical_bytes = document.bitcode_bytes()?;
        let sha256 = Digest32::digest_bytes(&canonical_bytes);
        Ok(Self {
            document,
            canonical_bytes,
            sha256,
        })
    }
}

/// Exact ordered static-content projection for a prepared mission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulationContentProjectionV1 {
    components: Vec<ProjectedSimulationContentComponentV1>,
    ranked_timing_ready: bool,
    ranked_opacity_ready: bool,
}

impl SimulationContentProjectionV1 {
    pub fn components(&self) -> &[ProjectedSimulationContentComponentV1] {
        &self.components
    }

    pub fn component(
        &self,
        kind: SimulationContentComponentKindV1,
    ) -> &ProjectedSimulationContentComponentV1 {
        self.components
            .iter()
            .find(|component| component.document.kind == kind)
            .unwrap_or_else(|| panic!("prepared projection is missing required {kind:?} component"))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_prepared_engine_inputs(
        loaded_level: &LoadedLevel,
        assets: &LevelAssets,
        bg_pixel_dims: (f32, f32),
        ground_mark_sprite: Option<&GroundMarkSpriteData>,
        titbit_row_frame_counts: &[u16],
    ) -> Result<Self, ProjectionError> {
        // Exhaustive destructuring is intentional. Adding a LevelAssets field
        // must force an explicit projection/derivation/exclusion decision.
        let LevelAssets {
            navigation:
                crate::engine::LevelNavigationAssets {
                    level_grid,
                    pathfinder_graph,
                    hiking_paths,
                    hiking_waypoint_sectors,
                    legacy_grid_topology,
                },
            environment:
                crate::engine::LevelEnvironmentAssets {
                    ambience_shadow_sectors,
                    water_zones,
                    material_sectors,
                    all_material_sectors,
                    static_sight_obstacles,
                },
            audio:
                crate::engine::LevelAudioAssets {
                    exclamation_durations,
                    speech_timing_catalog,
                    required_exclamation_ids,
                    source_durations,
                    sound_source_required_ids,
                },
            attachments:
                crate::engine::LevelRuntimeAttachments {
                    pixel_opacity,
                    // Executable attachment is excluded. Replay identity carries
                    // the canonical package and engine tape; ranked rejects it.
                    spellforge_runtime: _,
                },
            sprite_scriptor,
            profile_manager,
            bank_signature,
            scripts,
            entities: _, // Run-derived handles; raw authored inputs are in LoadedLevel.
            peasant_firstnames,
            peasant_surnames,
            fixed_vip_names,
            accessory_sprite_prototypes,
            character_sprite_prototypes: _, // Run-derived from campaign; sealed in run projection.
        } = assets;

        #[derive(Serialize)]
        struct LoadedLevelPayload<'a> {
            loaded_level: &'a LoadedLevel,
        }

        #[derive(Serialize)]
        struct MissionScriptsPayload<'a> {
            mission_programs:
                &'a BTreeMap<String, std::sync::Arc<crate::script_manager::ScriptProgram>>,
            mission_name: &'a Option<String>,
            spellforge_names: &'a crate::natives::ScriptNameBindings,
            location_count: usize,
            point_count: usize,
            location_positions: &'a [(f32, f32)],
            location_layers: &'a [u16],
            location_sectors: &'a [u16],
            location_sector_handles: &'a [Option<crate::position_interface::SectorHandle>],
            building_count: usize,
            hiking_path_count: usize,
            zone_grid_indices: &'a [u32],
        }

        #[derive(Serialize)]
        struct SpriteSimulationMetadataPayload<'a> {
            bank_signature: u32,
            accessory_sprite_prototypes:
                &'a std::collections::HashMap<crate::element::ObjectType, crate::sprite::Sprite>,
        }

        #[derive(Serialize)]
        struct MapGeometryMetadataPayload<'a> {
            bg_pixel_width_bits: u32,
            bg_pixel_height_bits: u32,
            level_grid: &'a crate::fast_find_grid::LevelGrid,
            pathfinder_graph: &'a crate::pathfinder::PathGraph,
            hiking_paths: &'a [crate::level_data::RawHikingPath],
            hiking_waypoint_sectors:
                &'a Option<std::sync::Arc<Vec<Vec<crate::position_interface::SectorHandle>>>>,
            legacy_grid_topology: &'a Option<crate::engine::LegacyGridTopologyAssets>,
            water_zones: &'a crate::water_zones::WaterZones,
            material_sectors: &'a crate::material_sectors::MaterialSectors,
            all_material_sectors: &'a [Option<crate::material_sectors::MaterialSector>],
            static_sight_obstacles: &'a [crate::sight_obstacle::SightObstacle],
            ambience_shadow_sectors: &'a [(crate::fast_find_grid::SectorIndex, u32)],
        }

        #[derive(Serialize)]
        struct LocalizedDeterministicTextPayload<'a> {
            peasant_firstnames: &'a [String],
            peasant_surnames: &'a [String],
            fixed_vip_names: &'a BTreeMap<String, String>,
        }

        #[derive(Serialize)]
        struct SoundDurationTablesPayload<'a> {
            exclamation_durations: &'a BTreeMap<(crate::sound::ExclamationGroup, u32, u16), u32>,
            speech_timing_catalog: &'a crate::engine::SpeechTimingCatalog,
            required_exclamation_ids: &'a std::collections::BTreeSet<u32>,
            source_durations: &'a BTreeMap<u32, u32>,
            sound_source_required_ids: &'a std::collections::BTreeSet<u32>,
        }

        #[derive(Serialize)]
        struct InterfaceSimulationMetadataPayload<'a> {
            ground_mark_sprite: Option<GroundMarkSpriteProjectionV1<'a>>,
            titbit_row_frame_counts: &'a [u16],
        }

        #[derive(Serialize)]
        struct GroundMarkSpriteProjectionV1<'a> {
            half_w_bits: u32,
            half_h_bits: u32,
            frame_sizes: &'a [(u16, u16)],
            per_frame_offsets: &'a [(i16, i16)],
        }

        let ground_mark_sprite = ground_mark_sprite.map(|ground| GroundMarkSpriteProjectionV1 {
            half_w_bits: ground.half_w.to_bits(),
            half_h_bits: ground.half_h.to_bits(),
            frame_sizes: &ground.frame_sizes,
            per_frame_offsets: &ground.per_frame_offsets,
        });

        let documents = [
            profiles_component_document_v1(profile_manager)?,
            component_document(
                SimulationContentComponentKindV1::LoadedLevel,
                LoadedLevelPayload { loaded_level },
            )?,
            component_document(
                SimulationContentComponentKindV1::MissionScripts,
                MissionScriptsPayload {
                    mission_programs: &scripts.mission_programs,
                    mission_name: &scripts.mission_name,
                    spellforge_names: &scripts.names,
                    location_count: scripts.location_count,
                    point_count: scripts.point_count,
                    location_positions: &scripts.location_positions,
                    location_layers: &scripts.location_layers,
                    location_sectors: &scripts.location_sectors,
                    location_sector_handles: &scripts.location_sector_handles,
                    building_count: scripts.building_count,
                    hiking_path_count: scripts.hiking_path_count,
                    zone_grid_indices: &scripts.zone_grid_indices,
                },
            )?,
            component_document(
                SimulationContentComponentKindV1::SpriteSimulationMetadata,
                SpriteSimulationMetadataPayload {
                    bank_signature: *bank_signature,
                    accessory_sprite_prototypes,
                },
            )?,
            component_document(
                SimulationContentComponentKindV1::MapGeometryMetadata,
                MapGeometryMetadataPayload {
                    bg_pixel_width_bits: bg_pixel_dims.0.to_bits(),
                    bg_pixel_height_bits: bg_pixel_dims.1.to_bits(),
                    level_grid,
                    pathfinder_graph,
                    hiking_paths,
                    hiking_waypoint_sectors,
                    legacy_grid_topology,
                    water_zones,
                    material_sectors,
                    all_material_sectors,
                    static_sight_obstacles,
                    ambience_shadow_sectors,
                },
            )?,
            component_document(
                SimulationContentComponentKindV1::LocalizedDeterministicText,
                LocalizedDeterministicTextPayload {
                    peasant_firstnames,
                    peasant_surnames,
                    fixed_vip_names,
                },
            )?,
            component_document(
                SimulationContentComponentKindV1::SoundDurationTables,
                SoundDurationTablesPayload {
                    exclamation_durations,
                    speech_timing_catalog,
                    required_exclamation_ids,
                    source_durations,
                    sound_source_required_ids,
                },
            )?,
            component_document(
                SimulationContentComponentKindV1::InterfaceSimulationMetadata,
                InterfaceSimulationMetadataPayload {
                    ground_mark_sprite,
                    titbit_row_frame_counts,
                },
            )?,
        ];

        let components = documents
            .into_iter()
            .map(ProjectedSimulationContentComponentV1::from_document)
            .collect::<Result<Vec<_>, _>>()?;
        debug_assert_eq!(
            components
                .iter()
                .map(|component| component.document.kind)
                .collect::<Vec<_>>(),
            SIMULATION_CONTENT_COMPONENT_ORDER_V1
        );
        let ranked_timing_ready = assets.audio.validate_ranked_timing().is_ok();
        let ranked_opacity_ready =
            pixel_opacity.is_some() && !sprite_scriptor.simulation_frame_ids().is_empty();
        Ok(Self {
            components,
            ranked_timing_ready,
            ranked_opacity_ready,
        })
    }
}

/// Run-specific projection sealed together with the static content projection.
///
/// The public highscore protocol wraps this information with the official
/// edition/subject/content-manifest/rules/speech identities. Keeping the exact
/// campaign, seed, config, and parity stream here prevents an Engine from being
/// created from values other than those inspected by preparation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedMissionRunProjectionV1 {
    pub schema_version: u32,
    pub static_components: Vec<PreparedStaticComponentIdentityV1>,
    pub starting_campaign: CanonicalSimulationValue,
    /// Digest of the exact opaque bitcode campaign bytes embedded in the
    /// replay header. This is the protocol's campaign-state identity; the
    /// canonical field above independently makes every constituent value
    /// inspectable when diagnosing a mismatch.
    pub starting_campaign_sha256: Digest32,
    pub starting_campaign_byte_length: u64,
    pub simulation_seed: SimulationSeed64,
    pub sim_config: CanonicalSimulationValue,
    pub original_rng_replay: Option<CanonicalSimulationValue>,
    pub original_rng_replay_sha256: Option<Digest32>,
    /// Run-derived dependency closure that cannot be published as static
    /// official content because campaign gang/team choices decide which
    /// character profiles and reinforcement prototypes are resolved.
    pub resolved_runtime_assets: CanonicalSimulationValue,
    /// Exact per-pixel hit-test behavior for every RHS frame reachable by the
    /// resolved closure. Encoded sprite/audio presentation bytes stay out.
    pub sprite_opacity_sha256: Option<Digest32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedStaticComponentIdentityV1 {
    pub kind: SimulationContentComponentKindV1,
    pub component_schema_version: u32,
    pub sha256: Digest32,
}

impl PreparedMissionRunProjectionV1 {
    pub(crate) fn new(
        static_projection: &SimulationContentProjectionV1,
        campaign: &Campaign,
        simulation_seed: u64,
        sim_config: &SimConfig,
        original_rng_replay: Option<&[u32]>,
        assets: &LevelAssets,
    ) -> Result<Self, ProjectionError> {
        #[derive(Serialize)]
        struct ResolvedRuntimeAssets<'a> {
            sprite_scriptor: &'a crate::sprite_script::SpriteScriptor,
            character_sprite_prototypes: &'a std::collections::HashMap<
                crate::profiles::CharacterProfileIdx,
                crate::sprite::Sprite,
            >,
            entity_bindings: &'a crate::engine::LevelEntityAssets,
        }

        let reachable_frame_ids = assets.sprite_scriptor.simulation_frame_ids();
        let sprite_opacity_sha256 = assets.attachments.pixel_opacity.as_ref().map(|opacity| {
            Digest32::from_bytes(opacity.simulation_opacity_sha256(&reachable_frame_ids))
        });
        let starting_campaign_bytes = bitcode::encode(campaign);
        Ok(Self {
            schema_version: PREPARED_MISSION_RUN_PROJECTION_SCHEMA_V1,
            static_components: static_projection
                .components()
                .iter()
                .map(|component| PreparedStaticComponentIdentityV1 {
                    kind: component.document.kind,
                    component_schema_version: component.document.component_schema_version,
                    sha256: component.sha256,
                })
                .collect(),
            starting_campaign: canonical_from_serializable(campaign)?,
            starting_campaign_sha256: Digest32::digest_bytes(&starting_campaign_bytes),
            starting_campaign_byte_length: u64::try_from(starting_campaign_bytes.len()).map_err(
                |_| {
                    ProjectionError::StaticContentMismatch(
                        "starting campaign byte length exceeds u64".to_owned(),
                    )
                },
            )?,
            simulation_seed: SimulationSeed64::new(simulation_seed),
            sim_config: canonical_from_serializable(sim_config)?,
            original_rng_replay: original_rng_replay
                .map(canonical_from_serializable)
                .transpose()?,
            original_rng_replay_sha256: original_rng_replay
                .map(bitcode::encode)
                .map(Digest32::digest_bytes),
            resolved_runtime_assets: canonical_from_serializable(&ResolvedRuntimeAssets {
                sprite_scriptor: &assets.sprite_scriptor,
                character_sprite_prototypes: &assets.character_sprite_prototypes,
                entity_bindings: &assets.entities,
            })?,
            sprite_opacity_sha256,
        })
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ProjectionError> {
        use robin_run_protocol::bitcode_value::BitcodeValue;
        // TODO: project typed inputs directly into ordered native fields to
        // eliminate the remaining serde_value tree during ranked preparation.
        let static_components = self
            .static_components
            .iter()
            .map(|component| {
                (
                    component.kind,
                    component.component_schema_version,
                    component.sha256,
                )
            })
            .collect::<Vec<_>>();
        Ok(bitcode::encode(&(
            *b"RHRP0002",
            self.schema_version,
            static_components,
            BitcodeValue::from_value(&self.starting_campaign)?,
            self.starting_campaign_sha256,
            self.starting_campaign_byte_length,
            self.simulation_seed.get(),
            BitcodeValue::from_value(&self.sim_config)?,
            self.original_rng_replay
                .as_ref()
                .map(BitcodeValue::from_value)
                .transpose()?,
            self.original_rng_replay_sha256,
            BitcodeValue::from_value(&self.resolved_runtime_assets)?,
            self.sprite_opacity_sha256,
        )))
    }

    pub fn sha256(&self) -> Result<Digest32, ProjectionError> {
        Ok(Digest32::digest_bytes(self.canonical_bytes()?))
    }

    pub fn is_rankable_input(&self) -> bool {
        self.original_rng_replay.is_none()
    }
}

/// Single-use mission preparation capability.
///
/// It is intentionally not `Clone`, not serializable, and has no public
/// constructor. Preparation constructs the engine exactly once, projects the
/// values actually consumed, and stores that engine behind this capability.
/// [`Engine::new`] consumes it; there is no opportunity to re-read or swap an
/// asset after its projection was inspected.
#[must_use = "prepared mission inputs must be consumed by Engine::new"]
pub struct PreparedMissionInputs {
    engine: Engine,
    static_projection: SimulationContentProjectionV1,
    run_projection: PreparedMissionRunProjectionV1,
}

impl PreparedMissionInputs {
    pub(crate) fn seal(
        engine: Engine,
        static_projection: SimulationContentProjectionV1,
        run_projection: PreparedMissionRunProjectionV1,
    ) -> Self {
        Self {
            engine,
            static_projection,
            run_projection,
        }
    }

    pub fn static_projection(&self) -> &SimulationContentProjectionV1 {
        &self.static_projection
    }

    pub fn run_projection(&self) -> &PreparedMissionRunProjectionV1 {
        &self.run_projection
    }

    pub fn run_projection_sha256(&self) -> Result<Digest32, ProjectionError> {
        self.run_projection.sha256()
    }

    /// Cross-bind the consumed engine-input projection to the official
    /// content and rules authorities selected by the ranked-session host.
    ///
    /// This does not read any assets: every simulation-affecting value is
    /// already behind this single-use capability. The returned protocol type
    /// is the same document independently recomputed by the verifier.
    pub fn protocol_seal(
        &self,
        bindings: PreparedMissionSealBindingsV1,
    ) -> Result<PreparedMissionInputsSealV1, ProjectionError> {
        let seal = PreparedMissionInputsSealV1 {
            schema_version: 1,
            prepared_inputs_projection_sha256: self.run_projection.sha256()?,
            content_manifest_sha256: bindings.content_manifest_sha256,
            content_edition: bindings.content_edition,
            content_subject: bindings.content_subject,
            starting_campaign_sha256: self.run_projection.starting_campaign_sha256,
            starting_campaign_byte_length: self.run_projection.starting_campaign_byte_length,
            simulation_seed: self.run_projection.simulation_seed,
            rules_config_sha256: bindings.rules_config_sha256,
            resource_locale_root: bindings.resource_locale_root,
            speech_timing: bindings.speech_timing,
            spellforge_content_sha256: bindings.spellforge_content_sha256,
            original_rng_replay_sha256: self.run_projection.original_rng_replay_sha256,
        };
        seal.validate()?;
        Ok(seal)
    }

    /// Fail-closed ranked admission. Missing source sound timing, parity RNG,
    /// Spellforge content, and malformed/zero protocol identities are errors,
    /// never silently downgraded to a valid ranked run.
    pub fn ranked_protocol_seal(
        &self,
        bindings: PreparedMissionSealBindingsV1,
    ) -> Result<PreparedMissionInputsSealV1, ProjectionError> {
        self.validate_rankable_timing()?;
        let seal = self.protocol_seal(bindings)?;
        seal.validate_rankable()?;
        Ok(seal)
    }

    /// Admit the prepared capability against the operator's already validated
    /// manifest and exact eight mounted component documents, then construct
    /// the ranked run seal. The local engine was built from the approved
    /// content mount before this call; byte equality here proves that its
    /// semantic projection is precisely the published one.
    pub fn admit_ranked_content(
        &self,
        manifest: &ContentManifestV1,
        mounted_documents: &[SimulationContentComponentDocumentV1],
        rules_config_sha256: Digest32,
        speech_timing: SpeechTimingAuthorityV1,
    ) -> Result<PreparedMissionInputsSealV1, ProjectionError> {
        self.validate_static_content(manifest, mounted_documents)?;
        self.ranked_protocol_seal(PreparedMissionSealBindingsV1 {
            content_manifest_sha256: manifest.canonical_digest()?,
            content_edition: manifest.edition,
            content_subject: manifest.subject.clone(),
            rules_config_sha256,
            resource_locale_root: manifest.resource_locale_root.clone(),
            speech_timing,
            spellforge_content_sha256: None,
        })
    }

    pub fn validate_static_content(
        &self,
        manifest: &ContentManifestV1,
        mounted_documents: &[SimulationContentComponentDocumentV1],
    ) -> Result<(), ProjectionError> {
        manifest.validate()?;
        if manifest.closure != ContentClosureKindV1::StaticPreparedMissionContentProjection
            || manifest.projection_schema_version != PREPARED_MISSION_RUN_PROJECTION_SCHEMA_V1
        {
            return Err(ProjectionError::StaticContentMismatch(
                "manifest names a different prepared-content projection schema".to_owned(),
            ));
        }
        if mounted_documents.len() != SIMULATION_CONTENT_COMPONENT_ORDER_V1.len() {
            return Err(ProjectionError::StaticContentMismatch(format!(
                "mounted catalog has {} documents, expected exactly 8",
                mounted_documents.len()
            )));
        }
        for (((expected_kind, local), mounted), reference) in SIMULATION_CONTENT_COMPONENT_ORDER_V1
            .into_iter()
            .zip(self.static_projection.components())
            .zip(mounted_documents)
            .zip(&manifest.components)
        {
            mounted.validate()?;
            let mounted_bytes = mounted.bitcode_bytes()?;
            let mounted_digest = Digest32::digest_bytes(&mounted_bytes);
            if local.document.kind != expected_kind
                || mounted.kind != expected_kind
                || reference.kind != expected_kind
                || local.document.component_schema_version != mounted.component_schema_version
                || mounted.component_schema_version != reference.component_schema_version
                || local.canonical_bytes != mounted_bytes
                || local.sha256 != mounted_digest
                || reference.artifact.sha256 != mounted_digest
                || reference.artifact.byte_length
                    != u64::try_from(mounted_bytes.len()).map_err(|_| {
                        ProjectionError::StaticContentMismatch(
                            "canonical component length exceeds u64".to_owned(),
                        )
                    })?
            {
                return Err(ProjectionError::StaticContentMismatch(format!(
                    "prepared engine content disagrees with mounted {expected_kind:?} artifact"
                )));
            }
        }
        Ok(())
    }

    fn validate_rankable_timing(&self) -> Result<(), ProjectionError> {
        if !self.static_projection.ranked_timing_ready {
            return Err(ProjectionError::IncompleteSpeechTiming);
        }
        if !self.static_projection.ranked_opacity_ready
            || self.run_projection.sprite_opacity_sha256.is_none()
        {
            return Err(ProjectionError::MissingSpriteOpacity);
        }
        Ok(())
    }

    pub(crate) fn into_engine(self) -> Engine {
        self.engine
    }
}

/// External signed authorities required to turn an engine preparation into a
/// protocol seal. These values do not originate inside `EngineArgs`, so the
/// caller must supply the exact manifest/rules/speech identities chosen before
/// simulation startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedMissionSealBindingsV1 {
    pub content_manifest_sha256: Digest32,
    pub content_edition: OfficialContentEditionV1,
    pub content_subject: OfficialContentSubjectV1,
    pub rules_config_sha256: Digest32,
    pub resource_locale_root: robin_run_protocol::ResourceLocaleRootV1,
    pub speech_timing: SpeechTimingAuthorityV1,
    pub spellforge_content_sha256: Option<Digest32>,
}

/// Validated operator inputs supplied to ranked preparation. The approved raw
/// datadir has already been resolved by the host into `EngineArgs`; these
/// references are the independently mounted semantic catalog used to admit
/// that exact preparation.
pub struct RankedContentAdmissionV1<'a> {
    pub manifest: &'a ContentManifestV1,
    pub mounted_documents: &'a [SimulationContentComponentDocumentV1],
    pub rules_config: &'a RulesConfigIdentityV1,
    pub speech_timing: SpeechTimingAuthorityV1,
}

/// Single-use ranked preparation capability. Preflight can inspect and sign
/// the seal, then [`Engine::new_ranked`] consumes this same owner. It is
/// intentionally neither `Clone` nor serializable and retains no content
/// path or loader.
#[must_use = "ranked prepared inputs must be consumed by Engine::new_ranked"]
pub struct RankedPreparedMissionInputs {
    prepared: PreparedMissionInputs,
    seal: PreparedMissionInputsSealV1,
    simulation_policy: RankedSimulationPolicy,
}

impl RankedPreparedMissionInputs {
    /// Admit an already-prepared, single-use engine owner against an operator
    /// catalog. Failure returns that exact owner so callers can continue the
    /// mission explicitly unranked without rebuilding or rereading content.
    pub fn admit(
        prepared: PreparedMissionInputs,
        admission: RankedContentAdmissionV1<'_>,
    ) -> Result<Self, (ProjectionError, PreparedMissionInputs)> {
        let (rules_sim_config, simulation_policy) =
            match validate_ranked_simulation_policy_rules_config_v1(admission.rules_config) {
                Ok(validated) => validated,
                Err(error) => return Err((error, prepared)),
            };
        if prepared.engine.sim_config() != rules_sim_config {
            return Err((
                ProjectionError::InvalidRankedSimulationPolicy(
                    "prepared engine SimConfig differs from canonical ranked policy".into(),
                ),
                prepared,
            ));
        }
        let rules_config_sha256 = match admission.rules_config.canonical_digest() {
            Ok(digest) => digest,
            Err(error) => return Err((ProjectionError::CanonicalDocument(error), prepared)),
        };
        match prepared.admit_ranked_content(
            admission.manifest,
            admission.mounted_documents,
            rules_config_sha256,
            admission.speech_timing,
        ) {
            Ok(seal) => Ok(Self {
                prepared,
                seal,
                simulation_policy,
            }),
            Err(error) => Err((error, prepared)),
        }
    }

    pub fn seal(&self) -> &PreparedMissionInputsSealV1 {
        &self.seal
    }

    pub fn static_projection(&self) -> &SimulationContentProjectionV1 {
        self.prepared.static_projection()
    }

    pub fn run_projection(&self) -> &PreparedMissionRunProjectionV1 {
        self.prepared.run_projection()
    }

    pub fn run_projection_sha256(&self) -> Result<Digest32, ProjectionError> {
        self.prepared.run_projection_sha256()
    }

    /// Consume ranked admission without starting a ranked session. This is
    /// the fail-closed escape hatch for signing/identity failures after local
    /// content admission and before frame zero.
    pub fn into_unranked(self) -> PreparedMissionInputs {
        self.prepared
    }

    pub(crate) fn into_prepared_and_policy(
        self,
    ) -> (PreparedMissionInputs, RankedSimulationPolicy) {
        (self.prepared, self.simulation_policy)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    #[error(transparent)]
    Bitcode(#[from] robin_run_protocol::bitcode_value::ProjectionBitcodeError),
    #[error("serialize deterministic input projection: {0}")]
    SerdeValue(#[from] serde_value::SerializerError),
    #[error(transparent)]
    Canonical(#[from] robin_run_protocol::CanonicalError),
    #[error(transparent)]
    CanonicalDocument(#[from] robin_run_protocol::CanonicalDocumentError),
    #[error(transparent)]
    Validation(#[from] robin_run_protocol::ValidationError),
    #[error("deterministic input map contains two keys with the same canonical identity")]
    DuplicateCanonicalMapKey,
    #[error("ranked preparation has missing speech or sound-source duration metadata")]
    IncompleteSpeechTiming,
    #[error("ranked preparation has no bound sprite pixel-opacity behavior")]
    MissingSpriteOpacity,
    #[error("static prepared-content admission failed: {0}")]
    StaticContentMismatch(String),
    #[error("invalid official projection rules config: {0}")]
    InvalidOfficialRulesConfig(String),
    #[error("invalid ranked simulation policy: {0}")]
    InvalidRankedSimulationPolicy(String),
    #[error("decode canonical deterministic input projection: {0}")]
    SerdeValueDecode(#[from] serde_value::DeserializerError),
    #[error("invalid canonical Profiles component: {0}")]
    InvalidProfilesComponent(String),
}

fn canonical_to_serde_value(
    value: &CanonicalSimulationValue,
) -> Result<serde_value::Value, ProjectionError> {
    use serde_value::Value;

    Ok(match value {
        CanonicalSimulationValue::Null => Value::Unit,
        CanonicalSimulationValue::Bool(value) => Value::Bool(*value),
        CanonicalSimulationValue::Signed(value) => Value::I64(*value),
        CanonicalSimulationValue::Unsigned(value) => Value::U64(*value),
        CanonicalSimulationValue::String(value) => Value::String(value.clone()),
        CanonicalSimulationValue::Array(values) => Value::Seq(
            values
                .iter()
                .map(canonical_to_serde_value)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        CanonicalSimulationValue::Object(values) => {
            if let Some(bits) = exact_unsigned_wrapper(values, "ieee754_f32_bits")? {
                let bits = u32::try_from(bits).map_err(|_| {
                    ProjectionError::InvalidProfilesComponent(
                        "ieee754_f32_bits exceeds u32".to_owned(),
                    )
                })?;
                Value::F32(f32::from_bits(bits))
            } else if let Some(bits) = exact_unsigned_wrapper(values, "ieee754_f64_bits")? {
                Value::F64(f64::from_bits(bits))
            } else if let Some(hex) = exact_string_wrapper(values, "bytes_hex")? {
                Value::Bytes(decode_lower_hex(hex)?)
            } else if let Some(entries) = exact_array_wrapper(values, "map_entries")? {
                let mut map = BTreeMap::new();
                for entry in entries {
                    let CanonicalSimulationValue::Object(entry) = entry else {
                        return Err(ProjectionError::InvalidProfilesComponent(
                            "canonical map entry is not an object".to_owned(),
                        ));
                    };
                    if entry.len() != 2
                        || !entry.contains_key("key")
                        || !entry.contains_key("value")
                    {
                        return Err(ProjectionError::InvalidProfilesComponent(
                            "canonical map entry must contain exactly key and value".to_owned(),
                        ));
                    }
                    let key = canonical_to_serde_value(&entry["key"])?;
                    let item = canonical_to_serde_value(&entry["value"])?;
                    if map.insert(key, item).is_some() {
                        return Err(ProjectionError::InvalidProfilesComponent(
                            "canonical map contains a duplicate typed key".to_owned(),
                        ));
                    }
                }
                Value::Map(map)
            } else {
                Value::Map(
                    values
                        .iter()
                        .map(|(key, value)| {
                            Ok((Value::String(key.clone()), canonical_to_serde_value(value)?))
                        })
                        .collect::<Result<BTreeMap<_, _>, ProjectionError>>()?,
                )
            }
        }
    })
}

fn exact_unsigned_wrapper(
    values: &BTreeMap<String, CanonicalSimulationValue>,
    key: &str,
) -> Result<Option<u64>, ProjectionError> {
    if values.len() != 1 || !values.contains_key(key) {
        return Ok(None);
    }
    match &values[key] {
        CanonicalSimulationValue::Unsigned(value) => Ok(Some(*value)),
        // Canonical JSON has one integer representation and the untagged
        // CanonicalValue decoder selects `Signed` first for every value that
        // fits i64. Accept that lossless wire representation while still
        // rejecting negative or non-integral bit patterns.
        CanonicalSimulationValue::Signed(value) if *value >= 0 => Ok(Some(*value as u64)),
        _ => Err(ProjectionError::InvalidProfilesComponent(format!(
            "{key} wrapper is not an unsigned integer"
        ))),
    }
}

fn exact_string_wrapper<'a>(
    values: &'a BTreeMap<String, CanonicalSimulationValue>,
    key: &str,
) -> Result<Option<&'a str>, ProjectionError> {
    if values.len() != 1 || !values.contains_key(key) {
        return Ok(None);
    }
    match &values[key] {
        CanonicalSimulationValue::String(value) => Ok(Some(value)),
        _ => Err(ProjectionError::InvalidProfilesComponent(format!(
            "{key} wrapper is not a string"
        ))),
    }
}

fn exact_array_wrapper<'a>(
    values: &'a BTreeMap<String, CanonicalSimulationValue>,
    key: &str,
) -> Result<Option<&'a [CanonicalSimulationValue]>, ProjectionError> {
    if values.len() != 1 || !values.contains_key(key) {
        return Ok(None);
    }
    match &values[key] {
        CanonicalSimulationValue::Array(value) => Ok(Some(value)),
        _ => Err(ProjectionError::InvalidProfilesComponent(format!(
            "{key} wrapper is not an array"
        ))),
    }
}

fn decode_lower_hex(value: &str) -> Result<Vec<u8>, ProjectionError> {
    if !value.len().is_multiple_of(2) {
        return Err(ProjectionError::InvalidProfilesComponent(
            "bytes_hex has odd length".to_owned(),
        ));
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            fn nibble(value: u8) -> Option<u8> {
                match value {
                    b'0'..=b'9' => Some(value - b'0'),
                    b'a'..=b'f' => Some(value - b'a' + 10),
                    _ => None,
                }
            }
            let high = nibble(pair[0]);
            let low = nibble(pair[1]);
            high.zip(low)
                .map(|(high, low)| (high << 4) | low)
                .ok_or_else(|| {
                    ProjectionError::InvalidProfilesComponent(
                        "bytes_hex is not canonical lowercase hexadecimal".to_owned(),
                    )
                })
        })
        .collect()
}

fn canonicalize_serde_value(
    value: serde_value::Value,
) -> Result<CanonicalSimulationValue, ProjectionError> {
    use serde_value::Value;

    Ok(match value {
        Value::Bool(value) => CanonicalSimulationValue::Bool(value),
        Value::U8(value) => CanonicalSimulationValue::Unsigned(value.into()),
        Value::U16(value) => CanonicalSimulationValue::Unsigned(value.into()),
        Value::U32(value) => CanonicalSimulationValue::Unsigned(value.into()),
        Value::U64(value) => CanonicalSimulationValue::Unsigned(value),
        Value::I8(value) => CanonicalSimulationValue::Signed(value.into()),
        Value::I16(value) => CanonicalSimulationValue::Signed(value.into()),
        Value::I32(value) => CanonicalSimulationValue::Signed(value.into()),
        Value::I64(value) => CanonicalSimulationValue::Signed(value),
        Value::F32(value) => float_bits("ieee754_f32_bits", u64::from(value.to_bits())),
        Value::F64(value) => float_bits("ieee754_f64_bits", value.to_bits()),
        Value::Char(value) => CanonicalSimulationValue::String(value.to_string()),
        Value::String(value) => CanonicalSimulationValue::String(value),
        Value::Unit => CanonicalSimulationValue::Null,
        Value::Option(None) => CanonicalSimulationValue::Null,
        Value::Option(Some(value)) | Value::Newtype(value) => canonicalize_serde_value(*value)?,
        Value::Seq(values) => CanonicalSimulationValue::Array(
            values
                .into_iter()
                .map(canonicalize_serde_value)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Value::Bytes(bytes) => {
            let mut object = BTreeMap::new();
            object.insert(
                "bytes_hex".to_owned(),
                CanonicalSimulationValue::String(hex_bytes(&bytes)),
            );
            CanonicalSimulationValue::Object(object)
        }
        Value::Map(values) => canonicalize_map(values)?,
    })
}

fn float_bits(name: &str, bits: u64) -> CanonicalSimulationValue {
    CanonicalSimulationValue::Object(BTreeMap::from([(
        name.to_owned(),
        CanonicalSimulationValue::Unsigned(bits),
    )]))
}

fn canonicalize_map(
    values: BTreeMap<serde_value::Value, serde_value::Value>,
) -> Result<CanonicalSimulationValue, ProjectionError> {
    if values
        .keys()
        .all(|key| matches!(key, serde_value::Value::String(_)))
    {
        let mut object = BTreeMap::new();
        for (key, value) in values {
            let serde_value::Value::String(key) = key else {
                unreachable!("map key shape checked above")
            };
            if object
                .insert(key, canonicalize_serde_value(value)?)
                .is_some()
            {
                return Err(ProjectionError::DuplicateCanonicalMapKey);
            }
        }
        return Ok(CanonicalSimulationValue::Object(object));
    }

    let mut entries = Vec::with_capacity(values.len());
    for (key, value) in values {
        let key = canonicalize_serde_value(key)?;
        let value = canonicalize_serde_value(value)?;
        let sort_key =
            bitcode::encode(&robin_run_protocol::bitcode_value::BitcodeValue::from_value(&key)?);
        entries.push((sort_key, key, value));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(ProjectionError::DuplicateCanonicalMapKey);
    }
    let entries = entries
        .into_iter()
        .map(|(_, key, value)| {
            CanonicalSimulationValue::Object(BTreeMap::from([
                ("key".to_owned(), key),
                ("value".to_owned(), value),
            ]))
        })
        .collect();
    Ok(CanonicalSimulationValue::Object(BTreeMap::from([(
        "map_entries".to_owned(),
        CanonicalSimulationValue::Array(entries),
    )])))
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection_profiles() -> crate::profiles::ProfileManager {
        use crate::coordinates::{MoveBox, SpriteAnchor};
        use crate::profiles::{CharacterProfile, MissionProfile};

        let mut profiles = crate::profiles::ProfileManager::new();
        profiles.characters.push(CharacterProfile {
            profile_name: "Robin des bois".to_owned(),
            box_move: MoveBox::from_geo(crate::geo2d::BBox2D::from_coords(
                -0.0,
                -3.25,
                4.5,
                f32::from_bits(0x7fc0_1234),
            )),
            center: SpriteAnchor::new(120.5, -0.0),
            ..Default::default()
        });
        profiles.missions.push(MissionProfile::default());
        profiles
    }

    #[test]
    fn profiles_component_is_a_strict_reversible_typed_boundary() {
        let document = profiles_component_document_v1(&projection_profiles()).unwrap();
        let decoded = profile_manager_from_component_document_v1(&document).unwrap();
        assert_eq!(profiles_component_document_v1(&decoded).unwrap(), document);

        // Exercise the actual native wire boundary, including normalized
        // integer signedness, rather than only the in-memory projection.
        let wire = document.bitcode_bytes().unwrap();
        let from_wire: SimulationContentComponentDocumentV1 =
            SimulationContentComponentDocumentV1::from_bitcode(&wire).unwrap();
        let decoded = profile_manager_from_component_document_v1(&from_wire).unwrap();
        assert_eq!(
            profiles_component_document_v1(&decoded)
                .unwrap()
                .bitcode_bytes()
                .unwrap(),
            wire
        );
    }

    #[test]
    fn profiles_component_decoder_rejects_kind_payload_and_float_tampering() {
        let document = profiles_component_document_v1(&projection_profiles()).unwrap();

        let mut wrong_kind = document.clone();
        wrong_kind.kind = SimulationContentComponentKindV1::LoadedLevel;
        assert!(profile_manager_from_component_document_v1(&wrong_kind).is_err());

        let mut extra_payload = document.clone();
        let CanonicalSimulationValue::Object(payload) = &mut extra_payload.payload else {
            panic!("test Profiles payload must be an object")
        };
        payload.insert("substitution".to_owned(), CanonicalSimulationValue::Null);
        assert!(profile_manager_from_component_document_v1(&extra_payload).is_err());

        let mut malformed_float = document;
        let CanonicalSimulationValue::Object(payload) = &mut malformed_float.payload else {
            panic!("test Profiles payload must be an object")
        };
        let CanonicalSimulationValue::Object(manager) = payload
            .get_mut("profile_manager")
            .expect("test Profiles payload must contain profile_manager")
        else {
            panic!("test profile_manager must be an object")
        };
        let CanonicalSimulationValue::Array(characters) = manager
            .get_mut("characters")
            .expect("test ProfileManager must contain characters")
        else {
            panic!("test characters must be an array")
        };
        let CanonicalSimulationValue::Object(character) = &mut characters[0] else {
            panic!("test character must be an object")
        };
        let CanonicalSimulationValue::Object(center) = character
            .get_mut("center")
            .expect("test character must contain center")
        else {
            panic!("test center must be an object")
        };
        center.insert(
            "x".to_owned(),
            CanonicalSimulationValue::Object(BTreeMap::from([(
                "ieee754_f32_bits".to_owned(),
                CanonicalSimulationValue::String("not bits".to_owned()),
            )])),
        );
        assert!(profile_manager_from_component_document_v1(&malformed_float).is_err());
    }

    #[test]
    fn float_projection_preserves_ieee_bits_including_negative_zero_and_nan() {
        #[derive(Serialize)]
        struct Floats {
            finite: f32,
            negative_zero: f32,
            nan: f64,
        }
        let projected = canonical_from_serializable(&Floats {
            finite: 1.25,
            negative_zero: -0.0,
            nan: f64::from_bits(0x7ff8_0000_0000_1234),
        })
        .unwrap();
        let bytes = canonical_json_bytes(&projected).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(r#""ieee754_f32_bits":1067450368"#));
        assert!(text.contains(r#""ieee754_f32_bits":2147483648"#));
        assert!(text.contains(r#""ieee754_f64_bits":9221120237041095220"#));
    }

    #[test]
    fn component_order_is_complete_and_stable() {
        assert_eq!(SIMULATION_CONTENT_COMPONENT_ORDER_V1.len(), 8);
        let names = SIMULATION_CONTENT_COMPONENT_ORDER_V1
            .iter()
            .map(|kind| serde_json::to_string(kind).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                r#""profiles""#,
                r#""loaded_level""#,
                r#""mission_scripts""#,
                r#""sprite_simulation_metadata""#,
                r#""map_geometry_metadata""#,
                r#""localized_deterministic_text""#,
                r#""sound_duration_tables""#,
                r#""interface_simulation_metadata""#,
            ]
        );
    }

    #[test]
    fn canonical_map_supports_non_string_keys_without_json_key_loss() {
        let source = BTreeMap::from([((crate::sound::ExclamationGroup::Pc, 7_u32, 3_u16), 9)]);
        let projected = canonical_from_serializable(&source).unwrap();
        let text = String::from_utf8(canonical_json_bytes(&projected).unwrap()).unwrap();
        assert!(text.contains("map_entries"));
        assert!(text.contains(r#""Pc""#));
    }

    #[test]
    fn official_rules_config_requires_exact_complete_typed_sim_config() {
        let sim_config = RankedSimulationPolicy::standard_medium().expected_config();
        let CanonicalSimulationValue::Object(sim_config) =
            canonical_from_serializable(&sim_config).unwrap()
        else {
            panic!("SimConfig must serialize as a canonical object")
        };
        let baseline = RulesConfigIdentityV1 {
            schema_version: 1,
            replay_schema_version: crate::replay::REPLAY_SCHEMA_VERSION,
            ranked_simulation_policy: RankedSimulationPolicy::standard_medium().identity(),
            sim_config,
            rules: BTreeMap::from([("ranked".into(), CanonicalSimulationValue::Bool(true))]),
        };
        assert_eq!(
            validate_official_projection_rules_config_v1(&baseline).unwrap(),
            RankedSimulationPolicy::standard_medium().expected_config()
        );
        let authored_json = canonical_json_bytes(&baseline).unwrap();
        let decoded: RulesConfigIdentityV1 = serde_json::from_slice(&authored_json).unwrap();
        assert_eq!(
            validate_official_projection_rules_config_v1(&decoded).unwrap(),
            RankedSimulationPolicy::standard_medium().expected_config(),
            "canonical operator JSON must preserve the complete typed SimConfig authority"
        );

        use crate::player_profile::DifficultyLevel;
        for difficulty in [
            DifficultyLevel::Easy,
            DifficultyLevel::Medium,
            DifficultyLevel::Hard,
            DifficultyLevel::Legendary,
            DifficultyLevel::custom(DifficultyLevel::Medium.rules()).unwrap(),
        ] {
            let mut custom = RankedSimulationPolicy::standard_medium().expected_config();
            custom.difficulty = difficulty;
            custom.enable_unbinding = !custom.enable_unbinding;
            let rules = custom_rules_config_v1(&baseline, custom).unwrap();
            let (decoded, sealed) =
                validate_ranked_simulation_policy_rules_config_v1(&rules).unwrap();
            assert_eq!(decoded, custom);
            assert!(sealed.validate_config(custom).is_ok());
            custom.script_enabled = !custom.script_enabled;
            assert!(sealed.validate_config(custom).is_err());
            let mut missing = rules;
            missing.sim_config.remove("script_enabled");
            assert!(validate_ranked_simulation_policy_rules_config_v1(&missing).is_err());
        }

        let mut missing = baseline.clone();
        missing.sim_config.remove("script_enabled");
        assert!(validate_official_projection_rules_config_v1(&missing).is_err());

        let mut unknown = baseline.clone();
        unknown
            .sim_config
            .insert("user_override".into(), CanonicalSimulationValue::Bool(true));
        assert!(validate_official_projection_rules_config_v1(&unknown).is_err());

        let mut hard = baseline.clone();
        hard.sim_config.insert(
            "difficulty".into(),
            CanonicalSimulationValue::String("Hard".into()),
        );
        assert!(validate_official_projection_rules_config_v1(&hard).is_err());
    }

    #[test]
    fn canonical_mission_setup_accepts_recorded_demo_start_and_rejects_modified_state() {
        use crate::profiles::{CharacterProfile, MissionProfile, ProfileManager};
        let mut profiles = ProfileManager::new();
        for (index, name) in [
            "Robin des villes",
            "Robin des bois",
            "Petit Jean",
            "Lady Marianne",
            "Frere Tuck",
            "Ferris",
        ]
        .into_iter()
        .enumerate()
        {
            profiles.characters.push(CharacterProfile {
                index: index as u32,
                profile_name: name.into(),
                ..Default::default()
            });
        }
        profiles.missions.push(MissionProfile {
            mission_filename: "Dem_Lei_MP".into(),
            ..Default::default()
        });
        let config = RankedSimulationPolicy::standard_medium().expected_config();
        let CanonicalSimulationValue::Object(sim_config) =
            canonical_from_serializable(&config).unwrap()
        else {
            panic!("config must be an object")
        };
        let rules = RulesConfigIdentityV1 {
            schema_version: 1,
            replay_schema_version: crate::replay::REPLAY_SCHEMA_VERSION,
            ranked_simulation_policy: RankedSimulationPolicy::standard_medium().identity(),
            sim_config,
            rules: BTreeMap::from([("ranked".into(), CanonicalSimulationValue::Bool(true))]),
        };
        let document = profiles_component_document_v1(&profiles).unwrap();
        let files = crate::sbfile::SbFileSystem::new(std::sync::Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        ));
        let subject = OfficialContentSubjectV1::FieldMission {
            mission_id: "Dem_Lei_MP".into(),
        };
        let artifact = |campaign: &Campaign| {
            let bytes = bitcode::encode(campaign);
            robin_run_protocol::ArtifactRefV1 {
                sha256: Digest32::digest_bytes(&bytes),
                byte_length: bytes.len() as u64,
                media_type: robin_run_protocol::RANKED_CAMPAIGN_MEDIA_TYPE_V1.into(),
            }
        };
        let mut campaign = Campaign::from_profiles(&profiles, config.difficulty);
        campaign.reset(&profiles, config.difficulty);
        // The launch path selects the demo party and records its restart checkpoint.
        campaign.create_gang_from_pcs_with_file_exists(
            "RJMTF",
            &profiles,
            config.difficulty,
            |_| false,
        );
        campaign.add_all_to_mission_team();
        campaign.current_mission_idx = Some(0);
        campaign.snapshot_preselected_with_simulation(17, config);
        assert!(
            validate_canonical_mission_start_v1(
                &rules,
                &document,
                OfficialContentEditionV1::Demo,
                &subject,
                17,
                &artifact(&campaign),
                &files
            )
            .is_ok()
        );
        assert!(
            validate_canonical_mission_start_v1(
                &rules,
                &document,
                OfficialContentEditionV1::Demo,
                &subject,
                18,
                &artifact(&campaign),
                &files
            )
            .is_err()
        );
        campaign.characters[0].status.life_points += 1;
        assert!(
            validate_canonical_mission_start_v1(
                &rules,
                &document,
                OfficialContentEditionV1::Demo,
                &subject,
                17,
                &artifact(&campaign),
                &files
            )
            .is_err()
        );
        let unselected = canonical_fresh_campaign_artifact_v1(&rules, &document).unwrap();
        assert!(
            validate_canonical_mission_start_v1(
                &rules,
                &document,
                OfficialContentEditionV1::Demo,
                &subject,
                17,
                &unselected,
                &files
            )
            .is_err()
        );
    }

    #[test]
    fn six_ranked_rules_config_digests_are_stable() {
        use robin_run_protocol::CanonicalDocument as _;

        let policies = [
            RankedSimulationPolicy::standard_easy(),
            RankedSimulationPolicy::standard_medium(),
            RankedSimulationPolicy::standard_hard(),
            RankedSimulationPolicy::original_easy(),
            RankedSimulationPolicy::original_medium(),
            RankedSimulationPolicy::original_hard(),
        ];
        let observed = policies.map(|policy| {
            let CanonicalSimulationValue::Object(sim_config) =
                canonical_from_serializable(&policy.expected_config()).unwrap()
            else {
                panic!("SimConfig must canonicalize as an object")
            };
            RulesConfigIdentityV1 {
                schema_version: 1,
                replay_schema_version: crate::replay::REPLAY_SCHEMA_VERSION,
                ranked_simulation_policy: policy.identity(),
                sim_config,
                rules: BTreeMap::from([("ranked".into(), CanonicalSimulationValue::Bool(true))]),
            }
            .canonical_digest()
            .unwrap()
            .to_string()
        });
        // Release pins cover replay schema 32 and the exhaustive current
        // SimConfig, including the default-off background-patch reversal rule.
        assert_eq!(
            observed,
            [
                "cb72d7e8c162ccc42ee5b98a8fe7032f8f592841032ace32d071f7f79b954900",
                "e1cc9254fb0bb8eeb2b5fcbc5a1cf12379a66222337b7126d9e965df72feb664",
                "25f6a9081db4769c8a258221a312671ca1d94a544b5507a9f58345f9347c1f98",
                "9eb98a0ec5f8cdb76563d490d0591ede678a84b5586a2498ff95ed503a182d16",
                "2412d0036f2452648175dc826b22fb703d9e19fbdde72fccbc94ef1834353dc5",
                "3882e1e381becc6ef9077d132b7898e2ef61dc54eff3a5ebbeae0b5a26e2027a",
            ]
        );
    }
}
