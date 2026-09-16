//! Frozen campaign wire layout for replay schemas 43 through 48.
//! Decode before converting through named fields; binary fields cannot be dropped in place.

use anyhow::{Context, Result};
use robin_engine::campaign::{Campaign, CampaignPracticeReturn, CampaignValues, PcDescription};
use robin_engine::gameplay_config::ItemGameplayConfig;
use robin_engine::mission::{Mission, MissionStatus};
use robin_engine::player_profile::DifficultyLevel;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct LegacySimConfig {
    difficulty: DifficultyLevel,
    fix_hard_reaction_times: bool,
    enable_unbinding: bool,
    clean_hands_npc_kills_invalidate: bool,
    reusable_cloaks: bool,
    reversible_background_patches: bool,
    item_gameplay: ItemGameplayConfig,
    noise_distraction_feedback: bool,
    diplomacy: bool,
    npc_faction_wars: bool,
    more_combat_gestures: bool,
    gesture_quality_damage: bool,
    fog_of_war: bool,
    script_enabled: bool,
    highlander: bool,
    highlander2: bool,
    golden_eye: bool,
    ignore_default_loose: bool,
    bypass_fog_sprites_crash: bool,
    amount_of_speaking: u16,
    synchronous_pathfinding: bool,
    sherwood_trading: bool,
    enable_timed_missions: bool,
    enable_dynamic_ambience: bool,
}

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct LegacyCampaign<S> {
    values: CampaignValues,
    deeds: robin_engine::achievement::CampaignDeeds,
    ares: i8,
    missions: Vec<Mission>,
    accessible_mission_indices: Vec<usize>,
    pending_accessible_mission_indices: Vec<usize>,
    last_mission_idx: Option<usize>,
    current_mission_idx: Option<usize>,
    next_mission_idx: Option<usize>,
    blazon_mission_idx: Option<usize>,
    last_pseudo_mission_status: MissionStatus,
    last_pseudo_mission_id: u32,
    mission_attempt_sequence: u64,
    campaign_history_run_id: Option<u64>,
    history_replay_mission_idx: Option<usize>,
    practice_return_snapshot: Option<CampaignPracticeReturn>,
    characters: Vec<PcDescription>,
    gang_indices: Vec<usize>,
    reservist_indices: Vec<usize>,
    mission_team_indices: Vec<usize>,
    peasant_names: Vec<String>,
    reservists_are_back: bool,
    collected_relics: Vec<u32>,
    production_sectors: Vec<robin_engine::sector_production::SectorProduction>,
    pre_mission_snapshot: S,
    pre_mission_rng_seed: Option<u64>,
    pre_mission_sim_config: Option<LegacySimConfig>,
    pre_mission_was_preselected: bool,
}

type LegacyRoot = LegacyCampaign<Option<LegacyCampaign<()>>>;

pub(super) fn migrate(bytes: &[u8]) -> Result<Vec<u8>> {
    let legacy: LegacyRoot =
        bitcode::decode(bytes).context("decode schema-43..48 replay campaign")?;
    let mut value = serde_json::to_value(legacy)?;
    remove_obsolete_config(&mut value);
    if let Some(snapshot) = value.get_mut("pre_mission_snapshot") {
        remove_obsolete_config(snapshot);
    }
    let campaign: Campaign =
        serde_json::from_value(value).context("convert replay campaign to current schema")?;
    Ok(bitcode::encode(&campaign))
}

fn remove_obsolete_config(campaign: &mut serde_json::Value) {
    if let Some(config) = campaign
        .get_mut("pre_mission_sim_config")
        .and_then(|v| v.as_object_mut())
    {
        config.remove("bypass_fog_sprites_crash");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_root_and_restart_configs_without_changing_campaign_state() {
        let mut campaign = Campaign::default();
        campaign.peasant_names = vec!["retained name".into()];
        campaign.pre_mission_rng_seed = Some(1234567);
        campaign.pre_mission_sim_config = Some(robin_engine::engine::SimConfig {
            amount_of_speaking: 37,
            synchronous_pathfinding: true,
            ..Default::default()
        });
        let mut expected = serde_json::to_value(&campaign).unwrap();
        let mut snapshot = expected.clone();
        snapshot["pre_mission_sim_config"]["amount_of_speaking"] = 91.into();
        snapshot["pre_mission_rng_seed"] = 7654321.into();
        expected["pre_mission_snapshot"] = snapshot;

        for bypass in [false, true] {
            let mut old = expected.clone();
            old["pre_mission_sim_config"]["bypass_fog_sprites_crash"] = bypass.into();
            old["pre_mission_snapshot"]["pre_mission_sim_config"]["bypass_fog_sprites_crash"] =
                (!bypass).into();
            let legacy: LegacyRoot = serde_json::from_value(old).unwrap();
            let bytes = bitcode::encode(&legacy);
            assert!(bitcode::decode::<Campaign>(&bytes).is_err());
            for version in 43..=48 {
                let mut header = serde_json::json!({"version": version, "campaign": bytes});
                super::super::upgrade_header(&mut header).unwrap();
                let migrated: Vec<u8> = serde_json::from_value(header["campaign"].clone()).unwrap();
                let restored: Campaign = bitcode::decode(&migrated).unwrap();
                assert_eq!(serde_json::to_value(restored).unwrap(), expected);
                let already_current = header.clone();
                super::super::upgrade_header(&mut header).unwrap();
                assert_eq!(header, already_current);
            }
        }
    }

    #[test]
    fn rejects_corrupt_campaign_instead_of_resetting_progress() {
        assert!(migrate(&[255, 255, 255]).is_err());
        let campaign = Campaign::default();
        let legacy: LegacyRoot =
            serde_json::from_value(serde_json::to_value(&campaign).unwrap()).unwrap();
        let bytes = bitcode::encode(&legacy);
        let restored: Campaign = bitcode::decode(&migrate(&bytes).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(restored).unwrap(),
            serde_json::to_value(campaign).unwrap()
        );
        let mut truncated = bytes;
        truncated.pop();
        assert!(migrate(&truncated).is_err());
    }
}
