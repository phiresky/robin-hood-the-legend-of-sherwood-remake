//! Frozen campaign configuration for replay schema 57.
use super::campaign_v48::LegacyCampaign;
use anyhow::{Context, Result};
use robin_engine::campaign::Campaign;
use robin_engine::gameplay_config::ItemGameplayConfig;
use robin_engine::player_profile::DifficultyLevel;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct LegacyCoopRules {
    players: u8,
    control: robin_engine::coop::CharacterControl,
    team: [u8; 5],
    duplicate_choices: [u8; 5],
    assignments: [u8; 5],
    enemy_health_per_duplicate: u16,
}

#[derive(Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
struct LegacySimConfig {
    coop: LegacyCoopRules,
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
    let legacy: LegacyRoot = bitcode::decode(bytes).context("decode schema-57 replay campaign")?;
    let campaign: Campaign = serde_json::from_value(serde_json::to_value(legacy)?)
        .context("add current configuration defaults to replay campaign")?;
    Ok(bitcode::encode(&campaign))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrates_root_and_restart_configs_with_exact_cooperative_party() {
        let mut campaign = Campaign::default();
        campaign.peasant_names = vec!["retained".into()];
        campaign.pre_mission_sim_config = Some(robin_engine::engine::SimConfig {
            coop: robin_engine::coop::CoopRules {
                players: 2,
                team: [82, 82, 0, 0, 0],
                assignments: [1, 0, 2, 3, 4],
                ..Default::default()
            },
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
        // Exercise the public normalization path for both standalone and archive headers.
        for chunk in [false, true] {
            let recording = serde_json::json!({
                "version": 57, "campaign": bytes,
                "rankability": {"status": "recorded", "taints": []}
            });
            let header = if chunk {
                serde_json::json!({"recording": recording})
            } else {
                recording
            };
            let records = "\n{\"f\":0,\"i\":{\"commands\":[]}}\n";
            let input = format!("{header}{records}");
            let (normalized, version) =
                super::super::normalize_jsonl(input.as_bytes(), chunk).unwrap();
            assert_eq!(version, 57);
            let normalized = String::from_utf8(normalized).unwrap();
            assert!(normalized.ends_with(records));
            let header: serde_json::Value =
                serde_json::from_str(normalized.lines().next().unwrap()).unwrap();
            let recording = if chunk { &header["recording"] } else { &header };
            assert_eq!(
                recording["version"],
                robin_engine::replay::REPLAY_SCHEMA_VERSION
            );
            assert_eq!(recording["rankability"]["taints"], serde_json::json!([]));
            let migrated: Vec<u8> = serde_json::from_value(recording["campaign"].clone()).unwrap();
            let restored: Campaign = bitcode::decode(&migrated).unwrap();
            assert_eq!(serde_json::to_value(restored).unwrap(), expected);
        }
        let migrated = migrate(&bytes).unwrap();
        let restored: Campaign = bitcode::decode(&migrated).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), expected);
    }
    #[test]
    fn rejects_corrupt_campaign() {
        assert!(migrate(&[255, 255, 255]).is_err());
    }
}
