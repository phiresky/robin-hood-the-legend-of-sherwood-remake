//! Private operator catalog and exact signed-request policy binding.
//!
//! The supervisor, rather than the uploader, selects this document. Every
//! public document embedded here is nevertheless content-addressed by the
//! co-signed submission. The only path is a read-only catalog whose resolved
//! mission directory contains the eight canonical simulation projections.

use std::io::{self, BufReader};
use std::path::{Component, Path, PathBuf};

use ed25519_dalek::{Signature, VerifyingKey};
use robin_run_protocol::{
    BuildManifestV1, CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1, CampaignSessionKindV1,
    CanonicalValue, Digest32, InitialStateExpectationV1, MAX_CAMPAIGN_SESSIONS_V1,
    OfficialContentEditionV1, OfficialContentSubjectV1, RulesConfigIdentityV1, RulesetBoardScopeV1,
    RunScopeKindV1, Validate as _, VerificationRequestV1,
    validate_official_ranked_scope_subject_v1,
};

use crate::content_manifest::{
    ManifestError, ValidatedContentMount, mount_is_read_only, validate_content_mount,
};

/// Hard ceiling applied before decoding the private per-job authority.
pub const MAX_JOB_CONFIG_BYTES: usize = robin_run_protocol::MAX_VERIFIER_JOB_CONFIG_BYTES_V1;

pub use robin_run_protocol::{CampaignSessionBindingV1, VerifierJobConfigV1};

/// Capability produced only after all operator documents, the signed request,
/// executing verifier artifact, and immutable eight-document mount agree.
#[derive(Debug)]
pub struct ValidatedJobConfig {
    config: VerifierJobConfigV1,
    content: ValidatedContentMount,
    raw_content: ValidatedRawContentMount,
    build_manifest_sha256: Digest32,
    campaign_content_manifest_sha256: Option<Digest32>,
    prepared_mission_inputs_seal_sha256: Digest32,
    rules_config_sha256: Digest32,
    ruleset_manifest_sha256: Digest32,
}

/// Symlink-free, read-only official source mount. This capability carries the
/// operator-authorized edition explicitly; filenames never infer edition.
#[derive(Debug)]
pub struct ValidatedRawContentMount {
    root: PathBuf,
    edition: OfficialContentEditionV1,
}

impl ValidatedRawContentMount {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn edition(&self) -> OfficialContentEditionV1 {
        self.edition
    }
}

impl ValidatedJobConfig {
    pub const fn config(&self) -> &VerifierJobConfigV1 {
        &self.config
    }

    pub const fn content(&self) -> &ValidatedContentMount {
        &self.content
    }

    pub const fn raw_content(&self) -> &ValidatedRawContentMount {
        &self.raw_content
    }

    pub const fn build_manifest_sha256(&self) -> Digest32 {
        self.build_manifest_sha256
    }

    pub const fn campaign_content_manifest_sha256(&self) -> Option<Digest32> {
        self.campaign_content_manifest_sha256
    }

    pub const fn prepared_mission_inputs_seal_sha256(&self) -> Digest32 {
        self.prepared_mission_inputs_seal_sha256
    }

    pub const fn rules_config_sha256(&self) -> Digest32 {
        self.rules_config_sha256
    }

    pub const fn ruleset_manifest_sha256(&self) -> Digest32 {
        self.ruleset_manifest_sha256
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JobConfigError {
    #[error("reading verifier job config failed: {0}")]
    Read(#[source] io::Error),
    #[error("verifier job config exceeds {limit} bytes")]
    TooLarge { limit: usize },
    #[error("verifier job config JSON is invalid: {0}")]
    Decode(String),
    #[error("unsupported verifier job config schema {0}")]
    UnsupportedSchema(u32),
    #[error("operator content root is not a normalized absolute path: `{0}`")]
    UnsafeContentRoot(PathBuf),
    #[error("operator content root contains a symlink or non-directory component: `{0}`")]
    UnsafeContentRootComponent(PathBuf),
    #[error("operator raw and projection content roots overlap")]
    OverlappingContentRoots,
    #[error("operator raw content root is not mounted read-only")]
    RawContentRootNotReadOnly,
    #[error("operator raw content tree contains a symlink or special file: `{0}`")]
    UnsafeRawContentEntry(PathBuf),
    #[error("operator raw content tree contains a writable nested mount: `{0}`")]
    WritableRawContentEntry(PathBuf),
    #[error("walking operator raw content tree failed at `{path}`: {message}")]
    RawContentWalk { path: PathBuf, message: String },
    #[error("operator raw content edition {actual:?} differs from signed edition {expected:?}")]
    RawContentEditionMismatch {
        expected: OfficialContentEditionV1,
        actual: OfficialContentEditionV1,
    },
    #[error("checking operator content root failed: {0}")]
    ContentRootIo(#[source] io::Error),
    #[error("operator document {document} is invalid: {message}")]
    InvalidDocument {
        document: &'static str,
        message: String,
    },
    #[error("operator document {document} has digest {actual}, signed request requires {expected}")]
    IdentityMismatch {
        document: &'static str,
        expected: Digest32,
        actual: Digest32,
    },
    #[error("running verifier artifact differs from the exact build manifest")]
    VerifierArtifactMismatch,
    #[error("ruleset does not admit the exact signed run tuple: {0}")]
    RulesetMismatch(&'static str),
    #[error("competition catalog does not match the signed offer: {0}")]
    CompetitionMismatch(String),
    #[error("campaign content catalog does not match the signed offer: {0}")]
    CampaignContentMismatch(&'static str),
    #[error("campaign session binding does not match signed run scope: {0}")]
    CampaignSessionMismatch(&'static str),
    #[error(transparent)]
    Content(#[from] ManifestError),
}

/// Read and recursively duplicate-check the private config under a hard
/// pre-decode byte ceiling.
pub fn read_job_config(path: &Path) -> Result<VerifierJobConfigV1, JobConfigError> {
    let metadata = std::fs::metadata(path).map_err(JobConfigError::Read)?;
    if metadata.len() > MAX_JOB_CONFIG_BYTES as u64 {
        return Err(JobConfigError::TooLarge {
            limit: MAX_JOB_CONFIG_BYTES,
        });
    }
    let bytes = std::fs::read(path).map_err(JobConfigError::Read)?;
    if bytes.len() > MAX_JOB_CONFIG_BYTES {
        return Err(JobConfigError::TooLarge {
            limit: MAX_JOB_CONFIG_BYTES,
        });
    }
    crate::strict_json::from_slice(&bytes)
        .map_err(|error| JobConfigError::Decode(error.to_string()))
}

/// Validate the catalog against an already authenticated request and the
/// exact executable performing verification.
pub fn validate_job_config(
    config: VerifierJobConfigV1,
    request: &VerificationRequestV1,
    verifier_executable: &Path,
) -> Result<ValidatedJobConfig, JobConfigError> {
    if config.schema_version != robin_run_protocol::SCHEMA_VERSION_V1 {
        return Err(JobConfigError::UnsupportedSchema(config.schema_version));
    }
    config
        .validate()
        .map_err(|error| invalid("verifier_job_config", error))?;
    let template = &config.template;
    validate_private_directory_root(&template.content_catalog_root)?;
    validate_private_directory_root(&template.raw_content_root)?;
    reject_overlapping_content_roots(&template.content_catalog_root, &template.raw_content_root)?;

    validate_document("build_manifest", &template.build_manifest)?;
    validate_document("content_manifest", &template.content_manifest)?;
    validate_document("rules_config", &template.rules_config)?;
    validate_document("ruleset_manifest", &template.ruleset_manifest)?;
    robin_engine::simulation_inputs::validate_ranked_simulation_policy_rules_config_v1(
        &template.rules_config,
    )
    .map_err(|error| invalid("rules_config.ranked_simulation_policy", error))?;
    if let Some(campaign_content) = &template.campaign_content_manifest {
        validate_document("campaign_content_manifest", campaign_content)?;
    }
    if let Some(competition) = &template.competition_manifest {
        validate_document("competition_manifest", competition)?;
    }

    let submission = &request.submission.submission;
    let offer = &submission.offer;
    let ranked = &offer.session_genesis.claim.ranked_session;
    if template.route != robin_run_protocol::VerifierJobRouteV1::from_request(request) {
        return Err(JobConfigError::RulesetMismatch("job_route"));
    }
    if template.raw_content_edition != ranked.content_edition {
        return Err(JobConfigError::RawContentEditionMismatch {
            expected: ranked.content_edition,
            actual: template.raw_content_edition,
        });
    }
    let build_manifest_sha256 = require_identity(
        "build_manifest",
        &template.build_manifest,
        offer.build_manifest_sha256,
    )?;
    let content_manifest_sha256 = require_identity(
        "content_manifest",
        &template.content_manifest,
        offer.content_manifest_sha256,
    )?;
    if config.expected_prepared_mission_inputs_seal_sha256
        != ranked.prepared_mission_inputs_seal_sha256
        || config.expected_prepared_inputs_projection_sha256
            != ranked.prepared_inputs_projection_sha256
    {
        return Err(JobConfigError::IdentityMismatch {
            document: "prepared_mission_inputs",
            expected: ranked.prepared_mission_inputs_seal_sha256,
            actual: config.expected_prepared_mission_inputs_seal_sha256,
        });
    }
    let prepared_mission_inputs_seal_sha256 = config.expected_prepared_mission_inputs_seal_sha256;
    let rules_config_sha256 = require_identity(
        "rules_config",
        &template.rules_config,
        offer.rules_config_sha256,
    )?;
    let ruleset_manifest_sha256 = require_identity(
        "ruleset_manifest",
        &template.ruleset_manifest,
        offer.ruleset_manifest_sha256,
    )?;
    config
        .template
        .ruleset_manifest
        .validate_ranked_simulation_policy(&config.template.rules_config)
        .map_err(|error| invalid("ruleset_manifest.ranked_simulation_policy", error))?;

    ranked
        .validate_content_manifest(&template.content_manifest)
        .map_err(|error| invalid("content_manifest/ranked_session", error))?;

    let campaign_content_manifest_sha256 =
        validate_campaign_content(&config, request, content_manifest_sha256)?;
    let build_semantics = template
        .build_manifest
        .backend_visible_v1()
        .map_err(|error| invalid("build_manifest/backend_visible_v1", error))?;
    validate_verifier_artifact(verifier_executable, &build_semantics)?;
    validate_ruleset_tuple(
        &config,
        request,
        build_manifest_sha256,
        content_manifest_sha256,
        campaign_content_manifest_sha256,
    )?;
    validate_campaign_state_binding(&config, request)?;
    validate_competition_tuple(&config, request)?;
    validate_campaign_session(&config, request)?;

    let content = validate_content_mount(
        &template.content_catalog_root,
        ranked.content_edition,
        &ranked.content_subject,
        &template.content_manifest,
        content_manifest_sha256,
    )?;
    validate_read_only_raw_content_tree(&template.raw_content_root)?;
    let raw_content = ValidatedRawContentMount {
        root: template.raw_content_root.clone(),
        edition: template.raw_content_edition,
    };

    Ok(ValidatedJobConfig {
        config,
        content,
        raw_content,
        build_manifest_sha256,
        campaign_content_manifest_sha256,
        prepared_mission_inputs_seal_sha256,
        rules_config_sha256,
        ruleset_manifest_sha256,
    })
}

fn invalid(document: &'static str, error: impl std::fmt::Display) -> JobConfigError {
    JobConfigError::InvalidDocument {
        document,
        message: error.to_string(),
    }
}

fn validate_document<T: robin_run_protocol::Validate>(
    document: &'static str,
    value: &T,
) -> Result<(), JobConfigError> {
    value.validate().map_err(|error| invalid(document, error))
}

fn require_identity<T: robin_run_protocol::CanonicalDocument>(
    document: &'static str,
    value: &T,
    expected: Digest32,
) -> Result<Digest32, JobConfigError> {
    let actual = value
        .canonical_digest()
        .map_err(|error| invalid(document, error))?;
    if actual != expected {
        return Err(JobConfigError::IdentityMismatch {
            document,
            expected,
            actual,
        });
    }
    Ok(actual)
}

fn validate_normalized_absolute_root(root: &Path) -> Result<(), JobConfigError> {
    let safe = root.is_absolute()
        && root
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)));
    if !safe || root.file_name().is_none() {
        return Err(JobConfigError::UnsafeContentRoot(root.to_path_buf()));
    }
    Ok(())
}

fn validate_private_directory_root(root: &Path) -> Result<(), JobConfigError> {
    validate_normalized_absolute_root(root)?;
    let mut current = PathBuf::from("/");
    for component in root.components() {
        match component {
            Component::RootDir => continue,
            Component::Normal(component) => current.push(component),
            _ => return Err(JobConfigError::UnsafeContentRoot(root.to_path_buf())),
        }
        let metadata =
            std::fs::symlink_metadata(&current).map_err(JobConfigError::ContentRootIo)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(JobConfigError::UnsafeContentRootComponent(current));
        }
    }
    Ok(())
}

fn reject_overlapping_content_roots(
    catalog_root: &Path,
    raw_root: &Path,
) -> Result<(), JobConfigError> {
    if catalog_root.starts_with(raw_root) || raw_root.starts_with(catalog_root) {
        return Err(JobConfigError::OverlappingContentRoots);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let catalog = std::fs::metadata(catalog_root).map_err(JobConfigError::ContentRootIo)?;
        let raw = std::fs::metadata(raw_root).map_err(JobConfigError::ContentRootIo)?;
        if (catalog.dev(), catalog.ino()) == (raw.dev(), raw.ino()) {
            return Err(JobConfigError::OverlappingContentRoots);
        }
    }
    Ok(())
}

/// Reject every filesystem escape below the supervisor-approved root. The
/// root mount being read-only is insufficient on its own: a nested bind mount
/// can have independent flags, and an internal symlink could redirect one of
/// the legacy case-insensitive loader lookups outside the approved datadir.
fn validate_read_only_raw_content_tree(root: &Path) -> Result<(), JobConfigError> {
    if !mount_is_read_only(root)? {
        return Err(JobConfigError::RawContentRootNotReadOnly);
    }
    for item in walkdir::WalkDir::new(root).follow_links(false) {
        let item = item.map_err(|error| JobConfigError::RawContentWalk {
            path: error.path().unwrap_or(root).to_path_buf(),
            message: error.to_string(),
        })?;
        let file_type = item.file_type();
        if file_type.is_symlink() || (!file_type.is_dir() && !file_type.is_file()) {
            return Err(JobConfigError::UnsafeRawContentEntry(
                item.path().to_path_buf(),
            ));
        }
        if !mount_is_read_only(item.path())? {
            return Err(JobConfigError::WritableRawContentEntry(
                item.path().to_path_buf(),
            ));
        }
    }
    Ok(())
}

fn validate_verifier_artifact(
    executable: &Path,
    build: &BuildManifestV1,
) -> Result<(), JobConfigError> {
    let metadata = std::fs::metadata(executable).map_err(JobConfigError::Read)?;
    if !metadata.is_file() || metadata.len() != build.verifier.byte_length {
        return Err(JobConfigError::VerifierArtifactMismatch);
    }
    let file = std::fs::File::open(executable).map_err(JobConfigError::Read)?;
    let digest = Digest32::digest_reader(BufReader::new(file)).map_err(JobConfigError::Read)?;
    if digest != build.verifier.sha256 {
        return Err(JobConfigError::VerifierArtifactMismatch);
    }
    Ok(())
}

fn validate_campaign_content(
    config: &VerifierJobConfigV1,
    request: &VerificationRequestV1,
    content_digest: Digest32,
) -> Result<Option<Digest32>, JobConfigError> {
    let offer = &request.submission.submission.offer;
    let ranked = &offer.session_genesis.claim.ranked_session;
    match (
        offer.starting_state.scope_kind(),
        ranked.campaign_content_manifest_sha256,
        &config.template.campaign_content_manifest,
    ) {
        (RunScopeKindV1::IndividualLevel, None, None) => Ok(None),
        (RunScopeKindV1::Campaign, Some(expected), Some(manifest)) => {
            let actual = require_identity("campaign_content_manifest", manifest, expected)?;
            if manifest.edition != ranked.content_edition {
                return Err(JobConfigError::CampaignContentMismatch("edition"));
            }
            if manifest.content_for(&ranked.content_subject) != Some(content_digest) {
                return Err(JobConfigError::CampaignContentMismatch(
                    "subject_content_manifest",
                ));
            }
            Ok(Some(actual))
        }
        _ => Err(JobConfigError::CampaignContentMismatch("scope_or_presence")),
    }
}

fn validate_ruleset_tuple(
    config: &VerifierJobConfigV1,
    request: &VerificationRequestV1,
    build_digest: Digest32,
    content_digest: Digest32,
    campaign_content_digest: Option<Digest32>,
) -> Result<(), JobConfigError> {
    let ruleset = &config.template.ruleset_manifest;
    let build = config
        .template
        .build_manifest
        .backend_visible_v1()
        .map_err(|error| invalid("build_manifest/backend_visible_v1", error))?;
    let submission = &request.submission.submission;
    let offer = &submission.offer;
    let genesis = &offer.session_genesis.claim;
    if ruleset.rules_config_sha256 != offer.rules_config_sha256 {
        return Err(JobConfigError::RulesetMismatch("rules_config_sha256"));
    }
    if ruleset.canonical_campaign_state != offer.starting_state.campaign_state_requirement() {
        return Err(JobConfigError::RulesetMismatch("canonical_campaign_state"));
    }
    if ruleset
        .allowed_build_manifest_sha256
        .binary_search(&build_digest)
        .is_err()
    {
        return Err(JobConfigError::RulesetMismatch("build_not_allowed"));
    }
    if ruleset
        .allowed_content_manifest_sha256
        .binary_search(&content_digest)
        .is_err()
    {
        return Err(JobConfigError::RulesetMismatch("content_not_allowed"));
    }
    if let Some(campaign_digest) = campaign_content_digest
        && ruleset
            .allowed_campaign_content_manifest_sha256
            .binary_search(&campaign_digest)
            .is_err()
    {
        return Err(JobConfigError::RulesetMismatch(
            "campaign_content_not_allowed",
        ));
    }
    let scope = match offer.starting_state.scope_kind() {
        RunScopeKindV1::IndividualLevel => RulesetBoardScopeV1::IndividualLevel,
        RunScopeKindV1::Campaign => RulesetBoardScopeV1::CampaignMission,
    };
    if ruleset.board_scopes.binary_search(&scope).is_err() {
        return Err(JobConfigError::RulesetMismatch("scope_not_allowed"));
    }
    let replay_schema = submission.artifacts.replay.replay_schema_version;
    if ruleset
        .replay_schema_versions
        .binary_search(&replay_schema)
        .is_err()
        || config.template.rules_config.replay_schema_version != replay_schema
        || build.replay_schema_version != replay_schema
    {
        return Err(JobConfigError::RulesetMismatch("replay_schema"));
    }
    if !is_current_ranked_network_tuple(
        &ruleset.network_protocol_versions,
        build.network_protocol_version,
        genesis.network_protocol_version,
    ) {
        return Err(JobConfigError::RulesetMismatch("network_protocol"));
    }
    if submission
        .requested_metrics
        .iter()
        .any(|metric| ruleset.metrics.binary_search(metric).is_err())
    {
        return Err(JobConfigError::RulesetMismatch("metric_not_allowed"));
    }
    let eligibility = &ruleset.participant_eligibility;
    let multiplayer = offer.max_concurrent_players > 1;
    if (multiplayer && !eligibility.allow_multiplayer)
        || (!multiplayer && !eligibility.allow_single_player)
        || offer.max_concurrent_players < eligibility.minimum_max_concurrent_players
        || offer.max_concurrent_players > eligibility.maximum_max_concurrent_players
        || offer.participant_instance_count > eligibility.maximum_participant_instances
    {
        return Err(JobConfigError::RulesetMismatch("participant_eligibility"));
    }
    Ok(())
}

fn is_current_ranked_network_tuple(
    ruleset_versions: &[u32],
    build_version: u32,
    genesis_version: u32,
) -> bool {
    ruleset_versions == [CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1]
        && build_version == CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1
        && genesis_version == CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1
}

fn validate_campaign_state_binding(
    config: &VerifierJobConfigV1,
    request: &VerificationRequestV1,
) -> Result<(), JobConfigError> {
    let submission = &request.submission.submission;
    let offer = &submission.offer;
    let pin = &config.template.canonical_campaign_state;
    if pin.requirement != offer.starting_state.campaign_state_requirement()
        || pin.requirement != config.template.ruleset_manifest.canonical_campaign_state
        || pin.requirement.rules_config_sha256 != offer.rules_config_sha256
        || pin.requirement.edition != offer.session_genesis.claim.ranked_session.content_edition
    {
        return Err(JobConfigError::RulesetMismatch(
            "canonical_campaign_state_binding",
        ));
    }
    match offer.starting_state {
        InitialStateExpectationV1::IndividualLevel { .. }
        | InitialStateExpectationV1::CampaignGenesis { .. } => {
            if pin.artifact != submission.artifacts.starting_campaign
                || pin.artifact.sha256 != offer.starting_state.campaign_sha256()
                || pin.artifact.byte_length != offer.starting_state.starting_campaign_byte_length()
            {
                return Err(JobConfigError::RulesetMismatch(
                    "canonical_campaign_state_artifact",
                ));
            }
        }
        InitialStateExpectationV1::CampaignContinuation { .. } => {
            // The current bytes are the verified predecessor output. The
            // signed requirement still fixes the canonical lineage genesis.
        }
    }
    Ok(())
}

fn validate_competition_tuple(
    config: &VerifierJobConfigV1,
    request: &VerificationRequestV1,
) -> Result<(), JobConfigError> {
    let offer = &request.submission.submission.offer;
    match (
        offer.competition_manifest_sha256,
        &config.template.competition_manifest,
    ) {
        (None, None) => Ok(()),
        (Some(expected), Some(competition)) => {
            require_identity("competition_manifest", competition, expected)?;
            competition
                .validate_submission_offer(offer)
                .map_err(|error| JobConfigError::CompetitionMismatch(error.to_string()))?;
            let grant = offer
                .session_genesis
                .claim
                .competition_run_grant
                .as_ref()
                .ok_or_else(|| {
                    JobConfigError::CompetitionMismatch(
                        "competition genesis has no pre-run grant".into(),
                    )
                })?;
            competition
                .validate_run_grant(grant)
                .map_err(|error| JobConfigError::CompetitionMismatch(error.to_string()))?;
            let key = VerifyingKey::from_bytes(grant.claim.grant_authority_public_key.as_bytes())
                .map_err(|_| {
                JobConfigError::CompetitionMismatch("invalid grant authority key".into())
            })?;
            let signature = Signature::from_bytes(grant.authority_signature.as_bytes());
            let bytes = grant.signing_bytes().map_err(|error| {
                JobConfigError::CompetitionMismatch(format!("grant canonicalization: {error}"))
            })?;
            key.verify_strict(&bytes, &signature).map_err(|_| {
                JobConfigError::CompetitionMismatch("grant signature authentication failed".into())
            })?;
            Ok(())
        }
        _ => Err(JobConfigError::CompetitionMismatch(
            "signed and operator competition presence differs".into(),
        )),
    }
}

fn validate_campaign_session(
    config: &VerifierJobConfigV1,
    request: &VerificationRequestV1,
) -> Result<(), JobConfigError> {
    let offer = &request.submission.submission.offer;
    let ranked = &offer.session_genesis.claim.ranked_session;
    validate_verifier_scope_subject(
        ranked.content_edition,
        &ranked.content_subject,
        &offer.starting_state,
    )?;
    let subject = &ranked.content_subject;
    match (&offer.starting_state, &config.campaign_session) {
        (InitialStateExpectationV1::IndividualLevel { .. }, None) => Ok(()),
        (InitialStateExpectationV1::CampaignGenesis { .. }, Some(binding))
            if binding.ordinal == 0 =>
        {
            validate_campaign_session_kind(&binding.kind, subject)
        }
        (InitialStateExpectationV1::CampaignContinuation { .. }, Some(binding))
            if binding.ordinal > 0 && binding.ordinal < MAX_CAMPAIGN_SESSIONS_V1 =>
        {
            validate_campaign_session_kind(&binding.kind, subject)
        }
        (InitialStateExpectationV1::IndividualLevel { .. }, Some(_)) => Err(
            JobConfigError::CampaignSessionMismatch("individual_has_campaign_session"),
        ),
        (
            InitialStateExpectationV1::CampaignGenesis { .. }
            | InitialStateExpectationV1::CampaignContinuation { .. },
            None,
        ) => Err(JobConfigError::CampaignSessionMismatch(
            "campaign_session_missing",
        )),
        (InitialStateExpectationV1::CampaignGenesis { .. }, Some(_)) => Err(
            JobConfigError::CampaignSessionMismatch("genesis_ordinal_must_be_zero"),
        ),
        (InitialStateExpectationV1::CampaignContinuation { .. }, Some(_)) => Err(
            JobConfigError::CampaignSessionMismatch("continuation_ordinal_out_of_range"),
        ),
    }
}

fn validate_verifier_scope_subject(
    edition: OfficialContentEditionV1,
    subject: &OfficialContentSubjectV1,
    starting_state: &InitialStateExpectationV1,
) -> Result<(), JobConfigError> {
    validate_official_ranked_scope_subject_v1(edition, subject, starting_state)
        .map_err(|_| JobConfigError::CampaignSessionMismatch("official_scope_subject"))
}

fn validate_campaign_session_kind(
    kind: &CampaignSessionKindV1,
    subject: &OfficialContentSubjectV1,
) -> Result<(), JobConfigError> {
    let matches = match (kind, subject) {
        (
            CampaignSessionKindV1::FieldMission {
                mission_id: session,
            },
            OfficialContentSubjectV1::FieldMission {
                mission_id: content,
            },
        ) => session == content,
        (
            CampaignSessionKindV1::Headquarters { hq_sequence },
            OfficialContentSubjectV1::Headquarters { .. },
        ) => *hq_sequence != 0,
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(JobConfigError::CampaignSessionMismatch(
            "kind_or_content_subject",
        ))
    }
}

/// Compare an engine `SimConfig` with the entire canonical map pinned by the
/// rules identity; adapters must not compare a hand-picked toggle subset.
pub fn sim_config_matches(
    observed: &robin_engine::engine::SimConfig,
    expected: &RulesConfigIdentityV1,
) -> Result<bool, serde_json::Error> {
    let observed = CanonicalValue::from_serializable(observed)?;
    Ok(observed == CanonicalValue::Object(expected.sim_config.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_campaign_genesis_state() -> InitialStateExpectationV1 {
        InitialStateExpectationV1::CampaignGenesis {
            template_id: robin_run_protocol::OpaqueId::new("full-campaign-genesis").unwrap(),
            campaign_state_requirement: robin_run_protocol::CanonicalCampaignStateRequirementV1 {
                edition: OfficialContentEditionV1::Full,
                kind: robin_run_protocol::CanonicalCampaignStateKindV1::FullCampaignGenesis,
                rules_config_sha256: Digest32::from_bytes([6; 32]),
            },
            campaign_sha256: Digest32::from_bytes([8; 32]),
            starting_campaign_byte_length: 321,
        }
    }

    #[test]
    fn private_content_roots_are_distinct_normalized_directories() {
        let catalog = tempfile::tempdir().unwrap();
        let raw = tempfile::tempdir().unwrap();
        assert!(validate_private_directory_root(catalog.path()).is_ok());
        assert!(validate_private_directory_root(raw.path()).is_ok());
        assert!(reject_overlapping_content_roots(catalog.path(), raw.path()).is_ok());

        let nested = catalog.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        assert!(matches!(
            reject_overlapping_content_roots(catalog.path(), &nested),
            Err(JobConfigError::OverlappingContentRoots)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn private_content_root_rejects_symlinked_components() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let actual = directory.path().join("actual");
        let link = directory.path().join("link");
        std::fs::create_dir(&actual).unwrap();
        symlink(&actual, &link).unwrap();
        assert!(matches!(
            validate_private_directory_root(&link),
            Err(JobConfigError::UnsafeContentRootComponent(_))
        ));
    }

    #[test]
    fn campaign_session_kind_is_exact_and_case_sensitive() {
        let subject = OfficialContentSubjectV1::FieldMission {
            mission_id: "MissionA".into(),
        };
        assert!(
            validate_campaign_session_kind(
                &CampaignSessionKindV1::FieldMission {
                    mission_id: "MissionA".into(),
                },
                &subject,
            )
            .is_ok()
        );
        assert!(
            validate_campaign_session_kind(
                &CampaignSessionKindV1::FieldMission {
                    mission_id: "missiona".into(),
                },
                &subject,
            )
            .is_err()
        );
    }

    #[test]
    fn verifier_session_rejects_h12_and_headquarters_as_full_campaign_genesis() {
        let genesis = full_campaign_genesis_state();
        assert!(
            validate_verifier_scope_subject(
                OfficialContentEditionV1::Full,
                &OfficialContentSubjectV1::FieldMission {
                    mission_id: robin_run_protocol::OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1
                        .to_owned(),
                },
                &genesis,
            )
            .is_ok()
        );
        for subject in [
            OfficialContentSubjectV1::FieldMission {
                mission_id: "H12_Not_MP".to_owned(),
            },
            OfficialContentSubjectV1::Headquarters {
                mission_id: robin_run_protocol::OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1.to_owned(),
            },
        ] {
            assert!(matches!(
                validate_verifier_scope_subject(OfficialContentEditionV1::Full, &subject, &genesis,),
                Err(JobConfigError::CampaignSessionMismatch(
                    "official_scope_subject"
                ))
            ));
        }
    }

    #[test]
    fn sim_config_comparison_covers_the_entire_serialized_shape() {
        let config =
            robin_engine::engine::RankedSimulationPolicy::standard_medium().expected_config();
        let CanonicalValue::Object(map) =
            CanonicalValue::from_serializable(&config).expect("serialize SimConfig")
        else {
            panic!("SimConfig must serialize as an object")
        };
        let expected = RulesConfigIdentityV1 {
            schema_version: 1,
            replay_schema_version: robin_engine::replay::REPLAY_SCHEMA_VERSION,
            ranked_simulation_policy: robin_run_protocol::RankedSimulationPolicyV1::standard(
                robin_run_protocol::RankedSimulationDifficultyV1::Medium,
            ),
            sim_config: map.clone(),
            rules: std::collections::BTreeMap::from([(
                "policy".into(),
                CanonicalValue::String("ranked".into()),
            )]),
        };
        assert!(sim_config_matches(&config, &expected).unwrap());

        let mut changed = expected;
        changed
            .sim_config
            .insert("script_enabled".into(), CanonicalValue::Bool(false));
        assert!(!sim_config_matches(&config, &changed).unwrap());
    }

    #[test]
    fn verifier_requires_one_exact_current_ranked_network_protocol() {
        let current = CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1;
        assert!(is_current_ranked_network_tuple(
            &[current],
            current,
            current
        ));
        assert!(!is_current_ranked_network_tuple(
            &[current - 1],
            current,
            current
        ));
        assert!(!is_current_ranked_network_tuple(
            &[current, current + 1],
            current,
            current
        ));
        assert!(!is_current_ranked_network_tuple(
            &[current],
            current - 1,
            current
        ));
        assert!(!is_current_ranked_network_tuple(
            &[current],
            current,
            current - 1
        ));
    }
}
