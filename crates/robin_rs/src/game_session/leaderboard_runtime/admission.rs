//! Select verifier documents for the conditions recorded in an uploaded replay.
use super::*;
use robin_run_protocol::{official_content_manifest_name_v1, official_content_subjects_v1};
pub(super) struct RankedMissionAuthority {
    pub(super) content_manifest: ContentManifestV1,
    pub(super) rules_config: RulesConfigIdentityV1,
    pub(super) published_ruleset: PublishedRulesetV1,
    pub(super) build_manifest_sha256: Digest32,
    pub(super) requested_metrics: Vec<BoardMetricV1>,
}
pub(super) async fn fetch_replay_authority(
    mission_id: &str,
    sim_config: robin_engine::engine::SimConfig,
    preferences: &LeaderboardPreferences,
) -> Result<RankedMissionAuthority, RankedError> {
    let api = LeaderboardApi::from_preferences(&preferences)
        .map_err(|error| RankedError::from(error).context("leaderboard endpoint unavailable"))?;
    let metadata = crate::leaderboard_service::decode_metadata(Ok(api.metadata()?.take().await?))?;
    let mission = metadata
        .missions
        .iter()
        .find(|mission| mission.mission_id == mission_id)
        .ok_or_else(|| {
            RankedError::unavailable(format!(
                "mission `{mission_id}` has no published ranked content"
            ))
        })?;
    let category = BoardCategoryV1::IndividualLevel;
    let candidate_facets = metadata
        .rulesets
        .iter()
        .filter(|ruleset| {
            ruleset.categories.contains(&category)
                && ruleset.content
                    == RunContentIdentityV1::Mission {
                        content_manifest_sha256: mission.content_manifest_sha256,
                    }
        })
        .collect::<Vec<_>>();
    if candidate_facets.is_empty() {
        return Err(RankedError::unavailable(format!(
            "no published ranked ruleset matches mission `{mission_id}`"
        )));
    }

    let content_task = api.content_manifest(mission.content_manifest_sha256)?;
    let content_manifest = crate::leaderboard_service::decode_content_manifest(
        Ok(content_task.take().await?),
        mission.content_manifest_sha256,
    )?;

    // A missing UI preference is not authority to pick the lexicographically
    // first board. Resolve every candidate against the exact loaded SimConfig
    // and the typed preset/difficulty identity, then require one unique match.
    let mut exact_matches = Vec::new();
    let mut rejected = Vec::new();
    for facet in candidate_facets {
        match fetch_and_validate_ruleset_candidate(
            &api,
            mission_id,
            sim_config,
            mission.content_manifest_sha256,
            facet,
            &content_manifest,
            RulesetBoardScopeV1::IndividualLevel,
            None,
        )
        .await
        {
            Ok(candidate) => exact_matches.push(candidate),
            Err(error) => rejected.push(format!(
                "{}/{}: {error}",
                facet.preset_id.as_str(),
                facet.difficulty_id.as_str()
            )),
        }
    }
    if exact_matches.iter().any(|(_, _, published)| {
        published.manifest.rules_config_constraint
            == robin_run_protocol::RulesConfigConstraintV1::ExactCanonicalDigestOnly
    }) {
        exact_matches.retain(|(_, _, published)| {
            published.manifest.rules_config_constraint
                == robin_run_protocol::RulesConfigConstraintV1::ExactCanonicalDigestOnly
        });
    }
    let [(ruleset, rules_config, published_ruleset)] = exact_matches.as_slice() else {
        return match exact_matches.len() {
            0 => Err(RankedError::unavailable(format!(
                "no published ranked facet exactly matches the loaded gameplay configuration ({})",
                rejected.join("; ")
            ))),
            count => Err(RankedError::unavailable(format!(
                "{count} published ranked facets match the loaded gameplay configuration; the server catalog must select a unique ruleset"
            ))),
        };
    };
    let build_manifest_sha256 = select_current_build(&api, published_ruleset).await?;
    Ok(RankedMissionAuthority {
        content_manifest,
        rules_config: rules_config.clone(),
        published_ruleset: published_ruleset.clone(),
        build_manifest_sha256,
        requested_metrics: ruleset.metrics.clone(),
    })
}
async fn fetch_and_validate_ruleset_candidate<'a>(
    api: &LeaderboardApi,
    mission_id: &str,
    sim_config: robin_engine::engine::SimConfig,
    expected_content_sha256: Digest32,
    facet: &'a robin_run_protocol::RulesetFacetV1,
    content_manifest: &ContentManifestV1,
    required_board_scope: RulesetBoardScopeV1,
    campaign_content_manifest_sha256: Option<Digest32>,
) -> Result<
    (
        &'a robin_run_protocol::RulesetFacetV1,
        RulesConfigIdentityV1,
        PublishedRulesetV1,
    ),
    RankedError,
> {
    let rules_task = api.rules_config(facet.rules_config_sha256)?;
    let published_task = api.published_ruleset(facet.ruleset_manifest_sha256)?;
    let rules_config = crate::leaderboard_service::decode_rules_config(
        Ok(rules_task.take().await?),
        facet.rules_config_sha256,
    )?;
    let published_ruleset = crate::leaderboard_service::decode_published_ruleset(
        Ok(published_task.take().await?),
        facet.ruleset_manifest_sha256,
    )?;
    let rules_config = if published_ruleset.manifest.rules_config_constraint
        == robin_run_protocol::RulesConfigConstraintV1::AnyCanonicalSimConfig
    {
        robin_engine::simulation_inputs::custom_rules_config_v1(&rules_config, sim_config)?
    } else {
        rules_config
    };
    validate_single_player_authority(
        mission_id,
        sim_config,
        expected_content_sha256,
        SinglePlayerAuthorityDocuments {
            facet,
            content: content_manifest,
            rules: &rules_config,
            published: &published_ruleset,
        },
        required_board_scope,
        campaign_content_manifest_sha256,
    )?;
    Ok((facet, rules_config, published_ruleset))
}

/// Server documents fetched for one ruleset facet, validated together
/// against the local mission tuple.
struct SinglePlayerAuthorityDocuments<'a> {
    facet: &'a robin_run_protocol::RulesetFacetV1,
    content: &'a ContentManifestV1,
    rules: &'a RulesConfigIdentityV1,
    published: &'a PublishedRulesetV1,
}

fn validate_single_player_authority(
    mission_id: &str,
    sim_config: robin_engine::engine::SimConfig,
    expected_content_sha256: Digest32,
    documents: SinglePlayerAuthorityDocuments<'_>,
    required_board_scope: RulesetBoardScopeV1,
    campaign_content_manifest_sha256: Option<Digest32>,
) -> Result<(), RankedError> {
    let SinglePlayerAuthorityDocuments {
        facet,
        content,
        rules,
        published,
    } = documents;
    let subject_is_official = official_content_subjects_v1(content.edition)
        .iter()
        .any(|subject| subject == &content.subject);
    if !subject_is_official
        || content.subject.mission_id() != mission_id
        || content.name != official_content_manifest_name_v1(content.edition, &content.subject)
        || content.canonical_digest()? != expected_content_sha256
    {
        return Err(RankedError::rejected(
            "content manifest is not the exact canonical official mission authority",
        ));
    }
    let expected_sim_config =
        robin_engine::simulation_inputs::validate_ranked_simulation_policy_rules_config_v1(rules)?
            .0;
    if expected_sim_config != sim_config {
        return Err(RankedError::rejected(
            "loaded mission SimConfig differs from the published ranked policy",
        ));
    }
    published
        .manifest
        .validate_ranked_simulation_policy(rules)?;
    if !matches!(
        published.operational_status,
        RulesetOperationalStatusV1::Active
    ) || published
        .manifest
        .allowed_content_manifest_sha256
        .binary_search(&expected_content_sha256)
        .is_err()
        || published
            .manifest
            .board_scopes
            .binary_search(&required_board_scope)
            .is_err()
        || campaign_content_manifest_sha256.is_some_and(|digest| {
            published
                .manifest
                .allowed_campaign_content_manifest_sha256
                .binary_search(&digest)
                .is_err()
        })
        || published.manifest.metrics != facet.metrics
        || published.manifest.rules_config_sha256 != facet.rules_config_sha256
        || published.manifest.preset_id != facet.preset_id
        || published.manifest.preset_name != facet.preset_name
        || published.manifest.difficulty_id != facet.difficulty_id
        || published.manifest.difficulty_name != facet.difficulty_name
        || !published
            .manifest
            .replay_schema_versions
            .contains(&robin_engine::replay::REPLAY_SCHEMA_VERSION)
        || !published
            .manifest
            .network_protocol_versions
            .contains(&robin_engine::multiplayer::NET_PROTOCOL_VERSION)
    {
        return Err(RankedError::rejected(
            "published ruleset does not admit the exact local mission tuple",
        ));
    }
    Ok(())
}

async fn select_current_build(
    api: &LeaderboardApi,
    published: &PublishedRulesetV1,
) -> Result<Digest32, RankedError> {
    for digest in &published.manifest.allowed_build_manifest_sha256 {
        let task = api.build_manifest(*digest)?;
        let build =
            crate::leaderboard_service::decode_build_manifest(Ok(task.take().await?), *digest)?;
        if build_matches_runtime(&build)? {
            return Ok(*digest);
        }
    }
    Err(RankedError::unavailable(
        "no allowlisted verifier build matches this replay and network version",
    ))
}

// Source and save-format identities describe artifacts; replay/network versions
// define client compatibility with an approved verifier.
pub(super) fn build_matches_runtime(build: &VersionedBuildManifest) -> Result<bool, RankedError> {
    let build = build.backend_visible_v1()?;
    Ok(
        build.replay_schema_version == robin_engine::replay::REPLAY_SCHEMA_VERSION
            && build.network_protocol_version == robin_engine::multiplayer::NET_PROTOCOL_VERSION,
    )
}

#[cfg(test)]
mod build_compatibility_tests {
    use super::*;
    use robin_run_protocol::{
        ArtifactRefV1, BuildManifestV1, NamedArtifactV1, ViewerArtifactRoleV1,
    };

    fn published_build() -> BuildManifestV1 {
        let artifact = |media: &str| ArtifactRefV1 {
            sha256: Digest32::from_bytes([1; 32]),
            byte_length: 1,
            media_type: media.into(),
        };
        BuildManifestV1 {
            schema_version: 1,
            // Deliberately not this client's source commit.
            source_commit: "a".repeat(40),
            cargo_lock_sha256: Digest32::from_bytes([2; 32]),
            target_triple: "wasm32-unknown-unknown".into(),
            cargo_profile: "wasm-release".into(),
            cargo_features: vec![],
            replay_schema_version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
            save_schema_version: crate::save_file::SAVE_FORMAT_VERSION + 1,
            network_protocol_version: robin_engine::multiplayer::NET_PROTOCOL_VERSION,
            verifier: artifact("application/x-executable"),
            viewer_artifacts: vec![
                NamedArtifactV1 {
                    path: "robin.js".into(),
                    role: ViewerArtifactRoleV1::EntryJavaScript,
                    artifact: artifact("text/javascript"),
                },
                NamedArtifactV1 {
                    path: "robin.wasm".into(),
                    role: ViewerArtifactRoleV1::WebAssembly,
                    artifact: artifact("application/wasm"),
                },
            ],
        }
    }

    #[test]
    fn compatible_versions_admit_other_commits_and_save_versions() {
        assert!(build_matches_runtime(&VersionedBuildManifest::V1(published_build())).unwrap());
    }

    #[test]
    fn replay_and_network_versions_must_both_match() {
        let mut replay = published_build();
        replay.replay_schema_version += 1;
        assert!(!build_matches_runtime(&VersionedBuildManifest::V1(replay)).unwrap());
        let mut network = published_build();
        network.network_protocol_version += 1;
        assert!(!build_matches_runtime(&VersionedBuildManifest::V1(network)).unwrap());
    }

    #[test]
    fn invalid_build_authority_still_errors() {
        let mut build = published_build();
        build.cargo_lock_sha256 = Digest32::from_bytes([0; 32]);
        assert!(build_matches_runtime(&VersionedBuildManifest::V1(build)).is_err());
    }
}
