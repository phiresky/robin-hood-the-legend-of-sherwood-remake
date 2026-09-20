//! Frozen campaign configuration for replay schemas 49 through 54.
use super::campaign_v48::LegacyCampaign;
use anyhow::{Context, Result};
use robin_engine::campaign::Campaign;
use robin_engine::gameplay_config::ItemGameplayConfig;
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
    amount_of_speaking: u16,
    synchronous_pathfinding: bool,
    sherwood_trading: bool,
    enable_timed_missions: bool,
    enable_dynamic_ambience: bool,
}

type LegacyRoot = LegacyCampaign<Option<LegacyCampaign<(), LegacySimConfig>>, LegacySimConfig>;

pub(super) fn migrate(bytes: &[u8]) -> Result<Vec<u8>> {
    let legacy: LegacyRoot =
        bitcode::decode(bytes).context("decode schema-49..54 replay campaign")?;
    let campaign: Campaign = serde_json::from_value(serde_json::to_value(legacy)?)
        .context("add cooperative defaults to replay campaign")?;
    Ok(bitcode::encode(&campaign))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrates_root_and_restart_configs_with_solo_defaults() {
        let mut campaign = Campaign::default();
        campaign.peasant_names = vec!["retained".into()];
        campaign.pre_mission_sim_config = Some(robin_engine::engine::SimConfig {
            amount_of_speaking: 37,
            ..Default::default()
        });
        let mut expected = serde_json::to_value(&campaign).unwrap();
        let mut snapshot = expected.clone();
        snapshot["pre_mission_sim_config"]["amount_of_speaking"] = 91.into();
        snapshot["pre_mission_rng_seed"] = 1234567.into();
        expected["pre_mission_snapshot"] = snapshot;
        let legacy: LegacyRoot = serde_json::from_value(expected.clone()).unwrap();
        let bytes = bitcode::encode(&legacy);
        assert!(bitcode::decode::<Campaign>(&bytes).is_err());
        for version in 49..=54 {
            let mut header = serde_json::json!({"version":version, "campaign":bytes});
            super::super::upgrade_header(&mut header).unwrap();
            let migrated: Vec<u8> = serde_json::from_value(header["campaign"].clone()).unwrap();
            let restored: Campaign = bitcode::decode(&migrated).unwrap();
            assert_eq!(serde_json::to_value(restored).unwrap(), expected);
            let current = header.clone();
            super::super::upgrade_header(&mut header).unwrap();
            assert_eq!(header, current);
        }
    }
    #[test]
    fn rejects_corrupt_campaign() {
        assert!(migrate(&[255, 255, 255]).is_err());
    }
}
