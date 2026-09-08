//! Private, typed authority exchanged between the ranked queue worker and the
//! one-job verifier. These documents are never public leaderboard data.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    CampaignContentManifestV1, CampaignSessionKindV1, CanonicalCampaignStatePinV1,
    CanonicalDocument as _, CompetitionManifestV1, ContentManifestV1, Digest32,
    ImmutablePolicyKindV1, ImmutablePolicyManifestV1, OfficialContentEditionV1,
    OfficialContentSubjectV1, RulesConfigIdentityV1, RulesetManifestV1, RunScopeKindV1, Validate,
    ValidationError, VerificationRequestV1, VersionedBuildManifest,
};

/// Shared pre-decode ceiling for both the private catalog artifact and the
/// exact per-job authority sealed into a verifier sandbox.
pub const MAX_VERIFIER_JOB_CONFIG_BYTES_V1: usize = 16 * 1024 * 1024;

/// Exact static tuple used to select one operator-authored verifier template.
/// Run-specific campaign and prepared-input identities are deliberately not
/// catalog keys: the worker injects them only after authenticating the request
/// and resolving the leased chain position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierJobRouteV1 {
    pub schema_version: u32,
    pub scope_kind: RunScopeKindV1,
    pub content_edition: OfficialContentEditionV1,
    pub content_subject: OfficialContentSubjectV1,
    pub build_manifest_sha256: Digest32,
    pub content_manifest_sha256: Digest32,
    pub campaign_content_manifest_sha256: Option<Digest32>,
    pub rules_config_sha256: Digest32,
    pub ruleset_manifest_sha256: Digest32,
    pub competition_manifest_sha256: Option<Digest32>,
}

impl VerifierJobRouteV1 {
    pub fn from_request(request: &VerificationRequestV1) -> Self {
        let offer = &request.submission.submission.offer;
        let ranked = &offer.session_genesis.claim.ranked_session;
        Self {
            schema_version: crate::SCHEMA_VERSION_V1,
            scope_kind: offer.starting_state.scope_kind(),
            content_edition: ranked.content_edition,
            content_subject: ranked.content_subject.clone(),
            build_manifest_sha256: offer.build_manifest_sha256,
            content_manifest_sha256: offer.content_manifest_sha256,
            campaign_content_manifest_sha256: ranked.campaign_content_manifest_sha256,
            rules_config_sha256: offer.rules_config_sha256,
            ruleset_manifest_sha256: offer.ruleset_manifest_sha256,
            competition_manifest_sha256: offer.competition_manifest_sha256,
        }
    }
}

impl Validate for VerifierJobRouteV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("VerifierJobRouteV1", self.schema_version)?;
        self.content_subject.validate()?;
        if [
            self.build_manifest_sha256,
            self.content_manifest_sha256,
            self.rules_config_sha256,
            self.ruleset_manifest_sha256,
        ]
        .into_iter()
        .any(|digest| digest.is_zero())
            || self
                .campaign_content_manifest_sha256
                .is_some_and(|digest| digest.is_zero())
            || self
                .competition_manifest_sha256
                .is_some_and(|digest| digest.is_zero())
        {
            return Err(ValidationError::Zero {
                field: "verifier_job_route.identity",
            });
        }
        let exact_official_lane = matches!(
            (
                self.content_edition,
                self.scope_kind,
                &self.content_subject,
                self.campaign_content_manifest_sha256,
            ),
            (
                OfficialContentEditionV1::Demo,
                RunScopeKindV1::IndividualLevel,
                OfficialContentSubjectV1::FieldMission { .. },
                None,
            ) | (
                OfficialContentEditionV1::Full,
                RunScopeKindV1::Campaign,
                OfficialContentSubjectV1::FieldMission { .. }
                    | OfficialContentSubjectV1::Headquarters { .. },
                Some(_),
            )
        );
        if !exact_official_lane {
            return Err(ValidationError::ClaimMismatch {
                field: "verifier_job_route.official_lane",
            });
        }
        Ok(())
    }
}

/// Static operator authority. One template may be used for arbitrarily many
/// jobs only when their complete authenticated static route is identical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierJobTemplateV1 {
    pub schema_version: u32,
    pub route: VerifierJobRouteV1,
    pub content_catalog_root: PathBuf,
    pub raw_content_root: PathBuf,
    pub raw_content_edition: OfficialContentEditionV1,
    pub build_manifest: VersionedBuildManifest,
    pub content_manifest: ContentManifestV1,
    pub campaign_content_manifest: Option<CampaignContentManifestV1>,
    pub rules_config: RulesConfigIdentityV1,
    pub ruleset_manifest: RulesetManifestV1,
    /// Operator-private exact starting state for this edition, scope, and
    /// complete rules configuration. Continuations retain this lineage pin
    /// while consuming the separately verified predecessor output.
    pub canonical_campaign_state: CanonicalCampaignStatePinV1,
    pub competition_manifest: Option<CompetitionManifestV1>,
}

/// Canonical private catalog pinned by an operator publication and loaded once
/// by the queue worker. Entries are ordered by the canonical route bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierJobConfigCatalogV1 {
    pub schema_version: u32,
    pub entries: Vec<VerifierJobTemplateV1>,
}

impl Validate for VerifierJobConfigCatalogV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("VerifierJobConfigCatalogV1", self.schema_version)?;
        if self.entries.is_empty() {
            return Err(ValidationError::Empty {
                field: "verifier_job_config_catalog.entries",
            });
        }
        let mut previous = None;
        for entry in &self.entries {
            entry.validate()?;
            let route = crate::canonical_json_bytes(&entry.route).map_err(|_| {
                ValidationError::ClaimMismatch {
                    field: "verifier_job_config_catalog.route",
                }
            })?;
            if previous
                .as_ref()
                .is_some_and(|value: &Vec<u8>| value >= &route)
            {
                return Err(ValidationError::NotCanonicalOrder {
                    field: "verifier_job_config_catalog.entries",
                });
            }
            previous = Some(route);
        }
        Ok(())
    }
}

/// Server-authoritative campaign-chain position. The public offer binds the
/// chain/predecessor; this value is derived from the accepted predecessor row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignSessionBindingV1 {
    pub kind: CampaignSessionKindV1,
    pub ordinal: u32,
}

/// Complete per-job configuration sealed into exactly one verifier process.
/// It cannot be used as the immutable verifier policy document: that policy is
/// independently selected from the manifest registry and cross-bound here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierJobConfigV1 {
    pub schema_version: u32,
    pub template: VerifierJobTemplateV1,
    pub verifier_policy_manifest: ImmutablePolicyManifestV1,
    pub expected_prepared_inputs_projection_sha256: Digest32,
    pub expected_prepared_mission_inputs_seal_sha256: Digest32,
    pub campaign_session: Option<CampaignSessionBindingV1>,
}

impl Validate for VerifierJobConfigV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("VerifierJobConfigV1", self.schema_version)?;
        self.template.validate()?;
        self.verifier_policy_manifest.validate()?;
        if self.expected_prepared_inputs_projection_sha256.is_zero()
            || self.expected_prepared_mission_inputs_seal_sha256.is_zero()
        {
            return Err(ValidationError::Zero {
                field: "verifier_job_config.prepared_inputs",
            });
        }
        let identity = &self.template.ruleset_manifest.verifier_policy;
        let policy_digest = self
            .verifier_policy_manifest
            .canonical_digest()
            .map_err(|_| ValidationError::ClaimMismatch {
                field: "verifier_job_config.verifier_policy_manifest",
            })?;
        if identity.kind != ImmutablePolicyKindV1::Verification
            || self.verifier_policy_manifest.kind != identity.kind
            || self.verifier_policy_manifest.version != identity.version
            || policy_digest != identity.manifest_sha256
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verifier_job_config.verifier_policy_manifest",
            });
        }
        Ok(())
    }
}

impl Validate for VerifierJobTemplateV1 {
    fn validate(&self) -> Result<(), ValidationError> {
        crate::validation::schema("VerifierJobTemplateV1", self.schema_version)?;
        self.route.validate()?;
        self.build_manifest.validate()?;
        self.content_manifest.validate()?;
        self.rules_config.validate()?;
        self.ruleset_manifest.validate()?;
        self.canonical_campaign_state.validate()?;
        if let Some(campaign) = &self.campaign_content_manifest {
            campaign.validate()?;
        }
        if let Some(competition) = &self.competition_manifest {
            competition.validate()?;
        }
        if self.raw_content_edition != self.route.content_edition
            || self.content_manifest.edition != self.route.content_edition
            || self.content_manifest.subject != self.route.content_subject
            || self.build_manifest.canonical_digest().ok() != Some(self.route.build_manifest_sha256)
            || self.content_manifest.canonical_digest().ok()
                != Some(self.route.content_manifest_sha256)
            || self.rules_config.canonical_digest().ok() != Some(self.route.rules_config_sha256)
            || self.ruleset_manifest.canonical_digest().ok()
                != Some(self.route.ruleset_manifest_sha256)
            || self.canonical_campaign_state.requirement
                != self.ruleset_manifest.canonical_campaign_state
            || self.canonical_campaign_state.requirement.edition != self.route.content_edition
            || self
                .canonical_campaign_state
                .requirement
                .rules_config_sha256
                != self.route.rules_config_sha256
            || self
                .campaign_content_manifest
                .as_ref()
                .map(|value| value.canonical_digest().ok())
                != self.route.campaign_content_manifest_sha256.map(Some)
            || self
                .competition_manifest
                .as_ref()
                .map(|value| value.canonical_digest().ok())
                != self.route.competition_manifest_sha256.map(Some)
            || self
                .competition_manifest
                .as_ref()
                .is_some_and(|competition| {
                    competition.canonical_campaign_state
                        != self.canonical_campaign_state.requirement
                        || competition.rules_config_sha256 != self.route.rules_config_sha256
                        || competition.ruleset_manifest_sha256 != self.route.ruleset_manifest_sha256
                })
        {
            return Err(ValidationError::ClaimMismatch {
                field: "verifier_job_template.route",
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(
        edition: OfficialContentEditionV1,
        scope_kind: RunScopeKindV1,
        content_subject: OfficialContentSubjectV1,
        campaign: bool,
    ) -> VerifierJobRouteV1 {
        VerifierJobRouteV1 {
            schema_version: crate::SCHEMA_VERSION_V1,
            scope_kind,
            content_edition: edition,
            content_subject,
            build_manifest_sha256: Digest32::from_bytes([1; 32]),
            content_manifest_sha256: Digest32::from_bytes([2; 32]),
            campaign_content_manifest_sha256: campaign.then_some(Digest32::from_bytes([3; 32])),
            rules_config_sha256: Digest32::from_bytes([4; 32]),
            ruleset_manifest_sha256: Digest32::from_bytes([5; 32]),
            competition_manifest_sha256: None,
        }
    }

    #[test]
    fn verifier_routes_admit_only_exact_demo_and_full_lanes() {
        let field = || OfficialContentSubjectV1::FieldMission {
            mission_id: "Dem_Lei_MP".into(),
        };
        let headquarters = || OfficialContentSubjectV1::Headquarters {
            mission_id: crate::OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1.into(),
        };
        assert!(
            route(
                OfficialContentEditionV1::Demo,
                RunScopeKindV1::IndividualLevel,
                field(),
                false,
            )
            .validate()
            .is_ok()
        );
        assert!(
            route(
                OfficialContentEditionV1::Full,
                RunScopeKindV1::Campaign,
                field(),
                true,
            )
            .validate()
            .is_ok()
        );
        assert!(
            route(
                OfficialContentEditionV1::Full,
                RunScopeKindV1::Campaign,
                headquarters(),
                true,
            )
            .validate()
            .is_ok()
        );
        for invalid in [
            route(
                OfficialContentEditionV1::Demo,
                RunScopeKindV1::Campaign,
                field(),
                true,
            ),
            route(
                OfficialContentEditionV1::Demo,
                RunScopeKindV1::IndividualLevel,
                headquarters(),
                false,
            ),
            route(
                OfficialContentEditionV1::Full,
                RunScopeKindV1::IndividualLevel,
                field(),
                false,
            ),
            route(
                OfficialContentEditionV1::Full,
                RunScopeKindV1::Campaign,
                field(),
                false,
            ),
        ] {
            assert!(invalid.validate().is_err());
        }
    }
}
