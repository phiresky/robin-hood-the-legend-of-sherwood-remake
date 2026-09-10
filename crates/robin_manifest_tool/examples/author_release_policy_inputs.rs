use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use robin_engine::engine::RankedSimulationPolicy;
use robin_run_protocol::{
    CanonicalValue, ImmutablePolicyKindV1, ImmutablePolicyManifestV1,
    OfficialProjectionAudioDurationPolicyV1, OfficialProjectionCampaignPolicyV1,
    OfficialProjectionDifficultyV1, OfficialProjectionExecutionPolicyV1,
    OfficialProjectionHostStatePolicyV1, OfficialProjectionLocalePolicyV1,
    OfficialProjectionOverlayPolicyV1, RulesConfigIdentityV1, SimulationSeed64, Validate,
    canonical_json_bytes,
};

fn value(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

fn rules(policy: RankedSimulationPolicy) -> RulesConfigIdentityV1 {
    RulesConfigIdentityV1 {
        schema_version: 1,
        replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        ranked_simulation_policy: policy.identity(),
        sim_config: match CanonicalValue::from_serializable(&policy.expected_config())
            .expect("serialize current SimConfig")
        {
            CanonicalValue::Object(values) => values,
            _ => unreachable!("SimConfig serializes as an object"),
        },
        rules: BTreeMap::from([
            ("canonical_replay_encoding".into(), value("compact_bitcode")),
            ("ranked".into(), CanonicalValue::Bool(true)),
            (
                "requires_complete_replay".into(),
                CanonicalValue::Bool(true),
            ),
            (
                "requires_server_resimulation".into(),
                CanonicalValue::Bool(true),
            ),
            ("terminal_result".into(), value("won")),
        ]),
    }
}

fn policy(
    kind: ImmutablePolicyKindV1,
    rules: &[(&str, CanonicalValue)],
) -> ImmutablePolicyManifestV1 {
    ImmutablePolicyManifestV1 {
        schema_version: 1,
        kind,
        version: 1,
        rules: rules
            .iter()
            .map(|(key, value)| ((*key).into(), value.clone()))
            .collect(),
    }
}

fn write<T: serde::Serialize>(path: PathBuf, document: &T) -> Result<()> {
    let bytes = canonical_json_bytes(document)?;
    fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))
}

#[derive(clap::Parser, serde::Serialize, serde::Deserialize)]
struct Args {
    output: PathBuf,
}

fn main() -> Result<()> {
    let output = <Args as clap::Parser>::parse().output;
    ensure!(
        !output.exists(),
        "output already exists: {}",
        output.display()
    );
    fs::create_dir_all(output.join("rules-configs"))?;
    fs::create_dir_all(output.join("policies"))?;

    let policies = [
        ("standard-easy", RankedSimulationPolicy::standard_easy()),
        ("standard-medium", RankedSimulationPolicy::standard_medium()),
        ("standard-hard", RankedSimulationPolicy::standard_hard()),
        (
            "original-parity-easy",
            RankedSimulationPolicy::original_easy(),
        ),
        (
            "original-parity-medium",
            RankedSimulationPolicy::original_medium(),
        ),
        (
            "original-parity-hard",
            RankedSimulationPolicy::original_hard(),
        ),
    ];
    let mut projection_rules = None;
    for (name, simulation_policy) in policies {
        let document = rules(simulation_policy);
        document.validate()?;
        robin_manifest_tool::validate_complete_ranked_rules_config_v1(&document)?;
        if name == "standard-medium" {
            robin_manifest_tool::validate_official_projection_rules_config_v1(&document)?;
            projection_rules = Some(document.clone());
        }
        write(
            output.join("rules-configs").join(format!("{name}.json")),
            &document,
        )?;
    }

    let projection_rules = projection_rules.context("missing Standard/Medium projection rules")?;
    let open_rules = robin_engine::simulation_inputs::custom_rules_config_v1(
        &projection_rules,
        RankedSimulationPolicy::standard_medium().expected_config(),
    )?;
    robin_manifest_tool::validate_complete_ranked_rules_config_v1(&open_rules)?;
    write(output.join("rules-configs/any.json"), &open_rules)?;

    let execution = OfficialProjectionExecutionPolicyV1 {
        schema_version: 1,
        policy_version: 1,
        difficulty: OfficialProjectionDifficultyV1::Medium,
        simulation_seed: SimulationSeed64::new(0),
        rules_config: projection_rules,
        campaign: OfficialProjectionCampaignPolicyV1::FreshRepresentativeCampaignPerOfficialSubjectV1,
        locale: OfficialProjectionLocalePolicyV1::ExactReceiptLcidBeforeResourceInitializationV1,
        audio_durations: OfficialProjectionAudioDurationPolicyV1::RebuildFromSourceClosureNoPersistentCacheV1,
        overlays: OfficialProjectionOverlayPolicyV1::BuiltInCoreOnlyRejectUserModEnvironmentV1,
        host_state: OfficialProjectionHostStatePolicyV1::CanonicalInMemoryNoPersistedPreferencesIdentitySaveOrEnvironmentV1,
    };
    execution.validate()?;
    write(output.join("projection-execution-policy.json"), &execution)?;

    let immutable = [
        policy(
            ImmutablePolicyKindV1::InputProvenance,
            &[
                ("accepted_encoding", value("compact_bitcode")),
                ("canonical_replay_schema_only", CanonicalValue::Bool(true)),
                ("console_input_allowed", CanonicalValue::Bool(false)),
                ("http_step_input_allowed", CanonicalValue::Bool(false)),
                ("playback_input_allowed", CanonicalValue::Bool(false)),
            ],
        ),
        policy(
            ImmutablePolicyKindV1::CommandAdmission,
            &[
                ("developer_commands_allowed", CanonicalValue::Bool(false)),
                ("mission_restart_allowed", CanonicalValue::Bool(false)),
                ("save_creation_allowed", CanonicalValue::Bool(false)),
                ("state_load_allowed", CanonicalValue::Bool(false)),
            ],
        ),
        policy(
            ImmutablePolicyKindV1::SubmissionAdmission,
            &[
                ("complete_replay_required", CanonicalValue::Bool(true)),
                (
                    "multiplayer_participant_cosignatures_required",
                    CanonicalValue::Bool(true),
                ),
                ("terminal_result", value("won")),
            ],
        ),
        policy(
            ImmutablePolicyKindV1::Verification,
            &[
                (
                    "exact_build_content_rules_campaign_required",
                    CanonicalValue::Bool(true),
                ),
                (
                    "isolated_server_resimulation_required",
                    CanonicalValue::Bool(true),
                ),
                ("metrics_recomputed_server_side", CanonicalValue::Bool(true)),
            ],
        ),
    ];
    for document in immutable {
        document.validate()?;
        let name = match document.kind {
            ImmutablePolicyKindV1::InputProvenance => "input-provenance",
            ImmutablePolicyKindV1::CommandAdmission => "command-admission",
            ImmutablePolicyKindV1::SubmissionAdmission => "submission-admission",
            ImmutablePolicyKindV1::Verification => "verification",
        };
        write(
            output.join("policies").join(format!("{name}.json")),
            &document,
        )?;
    }
    Ok(())
}
