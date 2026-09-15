//! Ranked board simulation policy and official fresh-start checks.
//!
//! The verifier resimulates uploaded replays from raw game content. This
//! module only maps a board's admitted simulation settings to the engine
//! policy and checks that an uploaded starting campaign is a genuine fresh
//! official start rather than a modified campaign.

use std::collections::BTreeMap;

use robin_run_types::{
    BoardSimulationPolicyV1, OfficialContentEditionV1, RankedSimulationDifficultyV1,
    RankedSimulationPolicyV1, RankedSimulationPresetV1,
};

use crate::campaign::Campaign;
use crate::engine::{Engine, RankedSimulationPolicy, RankedSimulationPolicyError, SimConfig};
use crate::player_profile::DifficultyLevel;
use crate::profiles::ProfileManager;

/// First mission selected by a freshly reset Full campaign.
pub const FULL_CAMPAIGN_GENESIS_MISSION_ID: &str = "H01_Lin_VL";

/// Resolve the engine policy a board applies to a replay recorded with
/// `observed`. Fixed boards admit exactly their preset configuration; any-config
/// boards admit every validated configuration under a `Custom` policy.
pub fn ranked_policy_for_board(
    board: BoardSimulationPolicyV1,
    observed: SimConfig,
) -> Result<RankedSimulationPolicy, RankedSimulationPolicyError> {
    match board {
        BoardSimulationPolicyV1::Fixed { policy } => {
            let policy = RankedSimulationPolicy::from_identity(policy)?;
            policy.validate_config(observed)?;
            Ok(policy)
        }
        BoardSimulationPolicyV1::AnyConfig => {
            let difficulty = match observed.difficulty {
                DifficultyLevel::Easy => RankedSimulationDifficultyV1::Easy,
                DifficultyLevel::Medium => RankedSimulationDifficultyV1::Medium,
                DifficultyLevel::Hard => RankedSimulationDifficultyV1::Hard,
                DifficultyLevel::Legendary => RankedSimulationDifficultyV1::Legendary,
                DifficultyLevel::Custom(_) => RankedSimulationDifficultyV1::Custom,
            };
            RankedSimulationPolicy::from_config(
                RankedSimulationPolicyV1 {
                    version: robin_run_types::RANKED_SIMULATION_POLICY_VERSION_V1,
                    preset: RankedSimulationPresetV1::Custom,
                    difficulty,
                },
                observed,
            )
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FreshStartError {
    #[error("official profiles cannot construct a fresh campaign")]
    IncompleteProfiles,
    #[error("official starting mission `{0}` is absent from profiles")]
    UnknownMission(String),
    #[error("unknown official demo starting team for mission `{0}`")]
    UnknownDemoTeam(String),
    #[error("cannot resolve official character file: {0}")]
    CharacterFile(String),
    #[error("starting campaign differs from every official fresh mission setup")]
    Mismatch,
}

/// Check that an uploaded starting campaign is exactly the pre-engine
/// checkpoint of an official fresh mission start, including the official team
/// and mission selection.
pub fn validate_fresh_mission_start(
    config: SimConfig,
    profiles: &ProfileManager,
    edition: OfficialContentEditionV1,
    mission_id: &str,
    simulation_seed: u64,
    starting_campaign: &[u8],
    files: &crate::sbfile::SbFileSystem,
) -> Result<(), FreshStartError> {
    if profiles.characters.len() < 2 || profiles.missions.is_empty() {
        return Err(FreshStartError::IncompleteProfiles);
    }
    let mission_index = profiles
        .missions
        .iter()
        .position(|mission| mission.mission_filename == mission_id)
        .ok_or_else(|| FreshStartError::UnknownMission(mission_id.to_owned()))?;
    let matches = |campaign: &Campaign| bitcode::encode(campaign) == starting_campaign;
    let mut fresh = Campaign::from_profiles(profiles, config.difficulty);
    fresh.reset(profiles, config.difficulty);
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
            let team = match mission_id {
                "Dem_Lei_MP" => "RJMTF",
                "Demo_Lin" => "RSABC",
                _ => return Err(FreshStartError::UnknownDemoTeam(mission_id.to_owned())),
            };
            let existing_files = profiles
                .characters
                .iter()
                .map(|profile| {
                    let path = format!("Data/Characters/{}.rhs", profile.filename);
                    files
                        .try_exists(&path)
                        .map(|exists| (path, exists))
                        .map_err(|status| FreshStartError::CharacterFile(status.to_string()))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            fresh.create_gang_from_pcs_with_file_exists(
                team,
                profiles,
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
            if mission_id == FULL_CAMPAIGN_GENESIS_MISSION_ID {
                // A new campaign begins with the application-owned seed zero.
                // Mission selection may advance it; both checkpoints must agree.
                fresh.snapshot_with_simulation(0, config);
                let (selected, selected_index, selected_seed, selected_config) =
                    Engine::select_next_mission(fresh, profiles, 0, config);
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
    Err(FreshStartError::Mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_policy_admits_exact_fixed_presets_and_any_valid_config() {
        let fixed = BoardSimulationPolicyV1::Fixed {
            policy: RankedSimulationPolicyV1::standard(RankedSimulationDifficultyV1::Medium),
        };
        let expected = RankedSimulationPolicy::standard_medium().expected_config();
        assert_eq!(
            ranked_policy_for_board(fixed, expected)
                .unwrap()
                .expected_config(),
            expected
        );
        let mut changed = expected;
        changed.enable_unbinding = !changed.enable_unbinding;
        assert!(ranked_policy_for_board(fixed, changed).is_err());

        let policy = ranked_policy_for_board(BoardSimulationPolicyV1::AnyConfig, changed).unwrap();
        assert_eq!(policy.identity().preset, RankedSimulationPresetV1::Custom);
        assert!(policy.validate_config(changed).is_ok());
        assert!(policy.validate_config(expected).is_err());
    }

    #[test]
    fn fresh_mission_setup_accepts_recorded_demo_start_and_rejects_modified_state() {
        use crate::profiles::{CharacterProfile, MissionProfile};
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
        let files = crate::sbfile::SbFileSystem::new(std::sync::Arc::new(
            robin_util::asset_fs::AssetVfs::new(),
        ));
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
        let check = |seed, bytes: &[u8]| {
            validate_fresh_mission_start(
                config,
                &profiles,
                OfficialContentEditionV1::Demo,
                "Dem_Lei_MP",
                seed,
                bytes,
                &files,
            )
        };
        assert!(check(17, &bitcode::encode(&campaign)).is_ok());
        assert!(check(18, &bitcode::encode(&campaign)).is_err());
        let unselected = Campaign::from_profiles(&profiles, config.difficulty);
        assert!(check(17, &bitcode::encode(&unselected)).is_err());
        campaign.characters[0].status.life_points += 1;
        assert!(check(17, &bitcode::encode(&campaign)).is_err());
    }
}
