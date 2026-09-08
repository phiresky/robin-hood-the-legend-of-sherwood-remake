use anyhow::Context as _;
use robin_run_protocol::{
    BoardMetricV1, BuildManifestV1, CampaignContentManifestV1, CanonicalCampaignStatePinV1,
    CanonicalDocument as _, CanonicalValue, CompetitionManifestV1, ContentManifestV1, Digest32,
    ImmutablePolicyManifestV1, OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1,
    OfficialContentEditionV1, OfficialContentSubjectV1, OpaqueId, PublishedRulesetV1,
    RulesConfigIdentityV1, RulesetBoardScopeV1, RulesetManifestV1, RulesetOperationalStatusV1,
    Validate as _, VersionedBuildManifest, official_full_campaign_completion_policy_v1,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::io::Read as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The API transport cap is the canonical compact codec's input cap. The API
/// uses its allocation-free lexical preflight, but must never invoke base64,
/// zstd, bitcode, or typed validation on hostile upload bytes.
pub const HARD_MAX_REPLAY_BYTES: u64 =
    robin_replay_format::DEFAULT_REPLAY_ADMISSION_LIMITS.max_input_bytes as u64;
pub const HARD_MAX_CAMPAIGN_BYTES: u64 = 64 * 1024 * 1024;
pub const HARD_MAX_METADATA_BYTES: usize = 256 * 1024;
pub const HARD_MAX_PAGE_SIZE: u32 = 100;
const HARD_MAX_OPERATOR_DOCUMENT_BYTES: u64 = 1024 * 1024;
pub const DEFAULT_RUNTIME_FENCE_DIRECTORY: &str =
    "/home/robinhood/.local/share/robin-highscores/runtime-fence";
const HARD_MAX_CANDIDATE_MANIFEST_DOCUMENTS: usize = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub database_path: PathBuf,
    /// Pre-provisioned kernel-lock authority shared read-only by the API,
    /// worker, and backup process. Runtime code never creates or repairs it.
    pub runtime_fence_directory: PathBuf,
    #[serde(skip)]
    pub(crate) allow_test_fence_provisioning: bool,
    pub replay_directory: PathBuf,
    /// Shared private content-addressed campaign store used by the API,
    /// verifier worker, and backup tooling. It must never be a public static
    /// directory.
    pub campaign_state_directory: PathBuf,
    pub cursor_secret_path: PathBuf,
    /// Dedicated Ed25519 seed for scheduled-run grants. It must never be
    /// reused as the cursor HMAC key or exposed to clients.
    pub competition_run_grant_secret_path: PathBuf,
    /// Dedicated Ed25519 seed for run pre-frame admission. It authorizes both
    /// canonical fresh starts and server-recognized campaign continuations,
    /// and must not be reused for competitions, cursors, or public identity.
    pub run_preflight_grant_secret_path: PathBuf,
    /// Dedicated HMAC authority for authenticated backup readiness envelopes.
    /// It is deliberately distinct from runtime pagination and signing keys,
    /// and is never part of a backup restore payload.
    pub backup_authority_hmac_secret_path: PathBuf,
    pub moderation_bearer_token_path: Option<PathBuf>,
    pub moderation_operator_id: String,
    #[serde(skip)]
    pub moderation_bearer_token: Option<std::sync::Arc<Vec<u8>>>,
    pub allowed_origins: Vec<String>,
    /// Only peers in these networks may supply X-Forwarded-For. A matching
    /// peer must supply exactly one canonical IP address or the request fails
    /// closed; untrusted peers' forwarding headers are ignored.
    pub trusted_proxy_cidrs: Vec<String>,
    pub challenge_requests_per_minute_per_ip: u32,
    pub abuse_reports_per_hour_per_ip: u32,
    pub abuse_reports_per_hour_per_key: u32,
    pub abuse_reports_per_hour_per_target: u32,
    /// Exact operator-installed official demo/full-retail configurations.
    /// Empty is a fail-closed configuration, not an allow-all wildcard.
    pub admission_profiles: Vec<AdmissionProfile>,
    pub competitions: Vec<CompetitionConfig>,
    /// Absolute root containing immutable canonical JSON documents in the
    /// documented per-kind subdirectories. Required when admission is enabled.
    pub manifest_directory: Option<PathBuf>,
    #[serde(skip)]
    pub manifests: std::sync::Arc<ManifestRegistry>,
    pub max_replay_bytes: u64,
    pub max_campaign_bytes: u64,
    pub max_metadata_bytes: usize,
    pub max_pending_submissions: u32,
    pub max_concurrent_requests: usize,
    pub max_concurrent_uploads: usize,
    pub upload_timeout_seconds: u64,
    /// Durable retry window for an upload whose signed metadata has already
    /// reserved and consumed its one-use challenge.
    pub upload_reservation_ttl_seconds: u64,
    pub max_page_size: u32,
    pub challenge_ttl_seconds: u64,
    /// A run-preflight grant must remain usable for the complete mission, while
    /// the short upload challenge is still minted only at mission end.
    pub run_preflight_ttl_seconds: u64,
    pub database_busy_timeout_ms: u64,
    pub tombstone_retention_days: Option<u64>,
    pub rejected_replay_retention_hours: u64,
    pub orphan_replay_retention_hours: u64,
    /// Free bytes which must remain after the full bounded admission plan.
    /// Production validation keeps this at or above one GiB.
    pub minimum_storage_free_bytes: u64,
    pub backup_manifest_path: Option<PathBuf>,
    /// Exact canonical manifest for the installed immutable release. Backup
    /// status is ready only when it binds this release identity.
    pub release_manifest_path: Option<PathBuf>,
    pub maximum_backup_age_hours: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionProfile {
    pub id: String,
    pub content_subject: OfficialContentSubjectV1,
    pub mission_display_name: String,
    pub allowed_scopes: Vec<String>,
    pub build_manifest_id: String,
    pub content_manifest_id: String,
    /// Required whenever this profile offers campaign play. This catalog binds
    /// the exact per-field/HQ content manifests used by a full campaign.
    pub campaign_content_manifest_id: Option<String>,
    pub config_id: String,
    pub ruleset_id: String,
    pub template_id: String,
    /// Exact deployment-private state pin for this profile's edition and
    /// rules configuration. The path is deliberately separate from the
    /// serializable artifact identity and may never be copied into public
    /// metadata or protocol proofs.
    pub canonical_campaign_state: CanonicalCampaignStatePinV1,
    pub canonical_campaign_state_path: PathBuf,
    pub allowed_metrics: Vec<String>,
    pub ruleset_display_name: String,
    pub preset_id: String,
    pub preset_name: String,
    pub difficulty_id: String,
    pub difficulty_name: String,
    pub build_display_name: String,
    pub viewer_engine_build: String,
    pub viewer_available: bool,
    pub viewer_unavailable_reason: Option<String>,
    pub viewer_content_requirement: Option<ViewerContentRequirementConfig>,
}

/// Operator-facing viewer entitlement. The value is deliberately separate
/// from the public launch DTO: its exact content digest comes from the
/// authenticated content manifest, never from mutable configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewerContentRequirementConfig {
    BundledDemo,
    UserLocalRetail,
}

impl ViewerContentRequirementConfig {
    pub(crate) const fn matches_edition(self, edition: OfficialContentEditionV1) -> bool {
        matches!(
            (edition, self),
            (
                OfficialContentEditionV1::Demo,
                ViewerContentRequirementConfig::BundledDemo
            ) | (
                OfficialContentEditionV1::Full,
                ViewerContentRequirementConfig::UserLocalRetail
            )
        )
    }
}

impl AdmissionProfile {
    pub fn mission_id(&self) -> &str {
        self.content_subject.mission_id()
    }
}

fn expected_admission_scopes(
    edition: OfficialContentEditionV1,
    subject: &OfficialContentSubjectV1,
) -> &'static [&'static str] {
    match (edition, subject) {
        (OfficialContentEditionV1::Demo, _) => &["individual_level"],
        (OfficialContentEditionV1::Full, OfficialContentSubjectV1::FieldMission { mission_id })
            if mission_id == OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1 =>
        {
            &["campaign_genesis", "campaign_continuation"]
        }
        (OfficialContentEditionV1::Full, _) => &["campaign_continuation"],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitionConfig {
    pub manifest_sha256: String,
    pub admission_profile_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct ManifestRegistry {
    /// Public build documents keyed by the canonical digest of the exact
    /// versioned document. `LoadedBuildManifest::semantics` is deliberately a
    /// separate value: its V1 digest is not the identity of a V2 document.
    pub builds: BTreeMap<Digest32, LoadedBuildManifest>,
    pub content_manifests: BTreeMap<Digest32, ContentManifestV1>,
    pub campaign_content_manifests: BTreeMap<Digest32, CampaignContentManifestV1>,
    pub rules_configs: BTreeMap<Digest32, RulesConfigIdentityV1>,
    pub rulesets: BTreeMap<Digest32, PublishedRulesetV1>,
    pub competitions: BTreeMap<Digest32, CompetitionManifestV1>,
    pub policies: BTreeMap<Digest32, ImmutablePolicyManifestV1>,
}

/// One immutable public build document and the normalized view consumed by
/// existing verifier/admission code. Construction validates both views and
/// records both identities so callers cannot accidentally substitute the V1
/// projection digest for a V2 public-document digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedBuildManifest {
    public_document: VersionedBuildManifest,
    public_digest: Digest32,
    semantics: BuildManifestV1,
    semantic_digest: Digest32,
}

impl LoadedBuildManifest {
    pub fn new(public_document: VersionedBuildManifest) -> anyhow::Result<Self> {
        public_document.validate()?;
        let public_digest = public_document.canonical_digest()?;
        let semantics = public_document.backend_visible_v1()?;
        semantics.validate()?;
        let semantic_digest = semantics.canonical_digest()?;
        Ok(Self {
            public_document,
            public_digest,
            semantics,
            semantic_digest,
        })
    }

    pub fn public_document(&self) -> &VersionedBuildManifest {
        &self.public_document
    }

    pub fn public_digest(&self) -> Digest32 {
        self.public_digest
    }

    pub fn semantics(&self) -> &BuildManifestV1 {
        &self.semantics
    }

    pub fn semantic_digest(&self) -> Digest32 {
        self.semantic_digest
    }
}

impl Serialize for LoadedBuildManifest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.public_document.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LoadedBuildManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let public_document = VersionedBuildManifest::deserialize(deserializer)?;
        Self::new(public_document).map_err(serde::de::Error::custom)
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787),
            database_path: PathBuf::from("data/highscores.sqlite3"),
            runtime_fence_directory: PathBuf::from(DEFAULT_RUNTIME_FENCE_DIRECTORY),
            allow_test_fence_provisioning: true,
            replay_directory: PathBuf::from("data/replays"),
            campaign_state_directory: PathBuf::from("data/campaign-states"),
            cursor_secret_path: PathBuf::from("data/cursor-hmac.key"),
            competition_run_grant_secret_path: PathBuf::from("data/competition-run-grant.key"),
            run_preflight_grant_secret_path: PathBuf::from("data/run-preflight-grant.key"),
            backup_authority_hmac_secret_path: PathBuf::from("data/backup-authority-hmac.key"),
            moderation_bearer_token_path: None,
            moderation_operator_id: "operator".to_owned(),
            moderation_bearer_token: None,
            allowed_origins: Vec::new(),
            trusted_proxy_cidrs: Vec::new(),
            challenge_requests_per_minute_per_ip: 120,
            abuse_reports_per_hour_per_ip: 10,
            abuse_reports_per_hour_per_key: 25,
            abuse_reports_per_hour_per_target: 10,
            admission_profiles: Vec::new(),
            competitions: Vec::new(),
            manifest_directory: None,
            manifests: std::sync::Arc::default(),
            max_replay_bytes: 16 * 1024 * 1024,
            max_campaign_bytes: 16 * 1024 * 1024,
            max_metadata_bytes: 64 * 1024,
            max_pending_submissions: 10_000,
            max_concurrent_requests: 256,
            max_concurrent_uploads: 32,
            upload_timeout_seconds: 120,
            upload_reservation_ttl_seconds: 30 * 60,
            max_page_size: 100,
            challenge_ttl_seconds: 10 * 60,
            run_preflight_ttl_seconds: 24 * 60 * 60,
            database_busy_timeout_ms: 5_000,
            tombstone_retention_days: Some(30),
            rejected_replay_retention_hours: 24,
            orphan_replay_retention_hours: 24,
            minimum_storage_free_bytes: 1024 * 1024 * 1024,
            backup_manifest_path: None,
            release_manifest_path: None,
            maximum_backup_age_hours: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigSecretScope {
    Api,
    Worker,
}

impl ServerConfig {
    /// Load the API configuration and its API-only moderation credential.
    ///
    /// Queue workers must use [`Self::load_for_worker`]. Keeping the entry
    /// points distinct is a security boundary: parsing the shared public
    /// configuration must not grant the worker a chance to open API secrets.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        Self::load_with_secret_scope(path, ConfigSecretScope::Api, &BTreeMap::new())
    }

    /// Load server configuration for the offline backup process, allowing
    /// systemd credential copies to stand in for API-owned mode-0400 files.
    /// The configured restore path remains unchanged; only the inode read by
    /// this process is substituted. Runtime services must use [`Self::load`].
    pub fn load_with_backup_credentials(
        path: &Path,
        credential_sources: &BTreeMap<PathBuf, PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::load_with_secret_scope(path, ConfigSecretScope::Api, credential_sources)
    }

    /// Load the public/runtime subset needed by the verification worker.
    ///
    /// The moderation-token path remains parsed so the worker validates the
    /// same operator document as the API, but it is deliberately never opened
    /// and the in-memory token is always `None`. Cursor, competition-grant,
    /// and run-preflight-grant keys are likewise opened only by their explicit
    /// API/admin methods.
    pub fn load_for_worker(path: &Path) -> anyhow::Result<Self> {
        Self::load_with_secret_scope(path, ConfigSecretScope::Worker, &BTreeMap::new())
    }

    fn load_with_secret_scope(
        path: &Path,
        scope: ConfigSecretScope,
        credential_sources: &BTreeMap<PathBuf, PathBuf>,
    ) -> anyhow::Result<Self> {
        let bytes = read_regular_file_no_symlinks(path, HARD_MAX_OPERATOR_DOCUMENT_BYTES)?;
        let mut config: Self = toml::from_str(std::str::from_utf8(&bytes)?)?;
        config.allow_test_fence_provisioning = false;
        config.manifests = match &config.manifest_directory {
            Some(root) => std::sync::Arc::new(ManifestRegistry::load(root)?),
            None => std::sync::Arc::default(),
        };
        config.moderation_bearer_token = match scope {
            ConfigSecretScope::Api => config
                .moderation_bearer_token_path
                .as_deref()
                .map(|configured| {
                    load_private_bearer_token(
                        credential_sources
                            .get(configured)
                            .map(PathBuf::as_path)
                            .unwrap_or(configured),
                    )
                })
                .transpose()?
                .map(std::sync::Arc::new),
            ConfigSecretScope::Worker => None,
        };
        config.validate_for_secret_scope(scope)?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        self.validate_for_secret_scope(ConfigSecretScope::Api)
    }

    fn validate_for_secret_scope(&self, scope: ConfigSecretScope) -> anyhow::Result<()> {
        self.validate_for_secret_scope_with_candidate(scope, None)
    }

    /// Validate a candidate's server document without resolving any of its
    /// immutable release paths through ambient pathnames. Deployment probes
    /// supply campaign bytes captured from the exact descriptors used by the
    /// authenticated structural scan instead.
    pub(crate) fn validate_for_runtime_probe(
        &self,
        authenticated_candidate_files: &BTreeMap<String, Vec<u8>>,
        source_commit: &str,
    ) -> anyhow::Result<()> {
        self.validate_for_secret_scope_with_candidate(
            ConfigSecretScope::Worker,
            Some((authenticated_candidate_files, source_commit)),
        )
    }

    fn validate_for_secret_scope_with_candidate(
        &self,
        scope: ConfigSecretScope,
        candidate: Option<(&BTreeMap<String, Vec<u8>>, &str)>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.max_replay_bytes > 0,
            "max_replay_bytes must be positive"
        );
        anyhow::ensure!(
            self.max_campaign_bytes > 0,
            "max_campaign_bytes must be positive"
        );
        anyhow::ensure!(
            self.tombstone_retention_days.is_none_or(|days| days >= 1),
            "tombstone retention must be at least one day"
        );
        anyhow::ensure!(
            (1..=24 * 30).contains(&self.rejected_replay_retention_hours),
            "rejected replay retention must be between one hour and 30 days"
        );
        anyhow::ensure!(
            (1..=24 * 30).contains(&self.orphan_replay_retention_hours),
            "orphan replay retention must be between one hour and 30 days"
        );
        anyhow::ensure!(
            self.minimum_storage_free_bytes
                >= crate::storage_admission::MINIMUM_STORAGE_RESERVE_BYTES,
            "minimum_storage_free_bytes must be at least 1 GiB"
        );
        anyhow::ensure!(
            self.backup_manifest_path.is_some() == self.maximum_backup_age_hours.is_some()
                && self.backup_manifest_path.is_some() == self.release_manifest_path.is_some(),
            "backup_manifest_path, release_manifest_path, and maximum_backup_age_hours must be configured together"
        );
        if let Some(hours) = self.maximum_backup_age_hours {
            anyhow::ensure!(
                hours == 32,
                "maximum backup age must be 32 hours to cover the daily schedule, jitter, timeout, and margin"
            );
        }
        if let Some(path) = &self.backup_manifest_path {
            anyhow::ensure!(path.is_absolute(), "backup_manifest_path must be absolute");
        }
        if let Some(path) = &self.release_manifest_path {
            anyhow::ensure!(path.is_absolute(), "release_manifest_path must be absolute");
            anyhow::ensure!(
                path.file_name().and_then(|name| name.to_str())
                    == Some("vps-release-manifest-v2.json"),
                "release_manifest_path must name vps-release-manifest-v2.json"
            );
        }
        anyhow::ensure!(
            self.max_replay_bytes <= HARD_MAX_REPLAY_BYTES,
            "max_replay_bytes exceeds the compiled safety limit of {HARD_MAX_REPLAY_BYTES}"
        );
        anyhow::ensure!(
            self.max_campaign_bytes <= HARD_MAX_CAMPAIGN_BYTES,
            "max_campaign_bytes exceeds the compiled safety limit of {HARD_MAX_CAMPAIGN_BYTES}"
        );
        anyhow::ensure!(
            self.max_metadata_bytes > 0 && self.max_metadata_bytes <= HARD_MAX_METADATA_BYTES,
            "max_metadata_bytes must be in 1..={HARD_MAX_METADATA_BYTES}"
        );
        anyhow::ensure!(
            self.max_pending_submissions > 0,
            "max_pending_submissions must be positive"
        );
        anyhow::ensure!(
            (1..=4_096).contains(&self.max_concurrent_requests),
            "max_concurrent_requests must be in 1..=4096"
        );
        anyhow::ensure!(
            (1..=1_024).contains(&self.max_concurrent_uploads),
            "max_concurrent_uploads must be in 1..=1024"
        );
        anyhow::ensure!(
            (10..=60 * 60).contains(&self.upload_timeout_seconds),
            "upload_timeout_seconds must be between 10 seconds and one hour"
        );
        anyhow::ensure!(
            self.upload_reservation_ttl_seconds
                >= self
                    .upload_timeout_seconds
                    .checked_add(30)
                    .ok_or_else(|| anyhow::anyhow!("upload timeout overflows"))?,
            "upload_reservation_ttl_seconds must exceed the upload timeout by at least 30 seconds"
        );
        anyhow::ensure!(
            self.upload_reservation_ttl_seconds <= 24 * 60 * 60,
            "upload reservation TTL must not exceed one day"
        );
        anyhow::ensure!(
            self.max_page_size > 0 && self.max_page_size <= HARD_MAX_PAGE_SIZE,
            "max_page_size must be in 1..={HARD_MAX_PAGE_SIZE}"
        );
        anyhow::ensure!(
            self.challenge_ttl_seconds >= 30,
            "challenge TTL is too short"
        );
        anyhow::ensure!(
            self.challenge_ttl_seconds <= Duration::from_secs(24 * 60 * 60).as_secs(),
            "challenge TTL must not exceed one day"
        );
        anyhow::ensure!(
            (60..=7 * 24 * 60 * 60).contains(&self.run_preflight_ttl_seconds),
            "run preflight TTL must be between one minute and seven days"
        );
        anyhow::ensure!(
            self.database_path.file_name().is_some(),
            "database_path must name a file"
        );
        anyhow::ensure!(
            self.runtime_fence_directory.is_absolute()
                && self
                    .runtime_fence_directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    == Some("runtime-fence"),
            "runtime_fence_directory must be an absolute path ending in runtime-fence"
        );
        anyhow::ensure!(
            self.replay_directory.file_name().is_some(),
            "replay_directory must not be a filesystem root"
        );
        anyhow::ensure!(
            self.campaign_state_directory.file_name().is_some(),
            "campaign_state_directory must not be a filesystem root"
        );
        anyhow::ensure!(
            self.replay_directory != self.campaign_state_directory,
            "replay and campaign stores must be distinct directories"
        );
        anyhow::ensure!(
            self.cursor_secret_path.file_name().is_some(),
            "cursor_secret_path must name a file"
        );
        anyhow::ensure!(
            self.competition_run_grant_secret_path.file_name().is_some()
                && self.competition_run_grant_secret_path != self.cursor_secret_path,
            "competition_run_grant_secret_path must name a distinct file"
        );
        anyhow::ensure!(
            self.run_preflight_grant_secret_path.file_name().is_some()
                && self.run_preflight_grant_secret_path != self.cursor_secret_path
                && self.run_preflight_grant_secret_path != self.competition_run_grant_secret_path,
            "run_preflight_grant_secret_path must name a distinct file"
        );
        anyhow::ensure!(
            self.backup_authority_hmac_secret_path.file_name().is_some()
                && self.backup_authority_hmac_secret_path != self.cursor_secret_path
                && self.backup_authority_hmac_secret_path != self.competition_run_grant_secret_path
                && self.backup_authority_hmac_secret_path != self.run_preflight_grant_secret_path
                && self
                    .moderation_bearer_token_path
                    .as_ref()
                    .is_none_or(|path| path != &self.backup_authority_hmac_secret_path),
            "backup_authority_hmac_secret_path must name a distinct file"
        );
        anyhow::ensure!(
            !self.moderation_operator_id.is_empty() && self.moderation_operator_id.len() <= 128,
            "moderation_operator_id must contain 1..=128 characters"
        );
        match scope {
            ConfigSecretScope::Api => anyhow::ensure!(
                self.moderation_bearer_token_path.is_some()
                    == self.moderation_bearer_token.is_some(),
                "moderation bearer token must be loaded exactly when its path is configured"
            ),
            ConfigSecretScope::Worker => anyhow::ensure!(
                self.moderation_bearer_token.is_none(),
                "worker configuration must not contain an API moderation credential"
            ),
        }
        for origin in &self.allowed_origins {
            let parsed = url::Url::parse(origin)
                .map_err(|error| anyhow::anyhow!("invalid CORS origin {origin}: {error}"))?;
            let http_loopback = parsed.scheme() == "http"
                && (parsed.host_str() == Some("localhost")
                    || parsed.host().is_some_and(|host| match host {
                        url::Host::Ipv4(address) => address == Ipv4Addr::LOCALHOST,
                        url::Host::Ipv6(address) => address == std::net::Ipv6Addr::LOCALHOST,
                        url::Host::Domain(_) => false,
                    }));
            anyhow::ensure!(
                parsed.scheme() == "https" || http_loopback,
                "allowed origin must use HTTPS (localhost HTTP is allowed): {origin}"
            );
            anyhow::ensure!(
                parsed.host_str().is_some()
                    && !parsed.host_str().is_some_and(|host| host.contains('*'))
                    && parsed.username().is_empty()
                    && parsed.password().is_none()
                    && parsed.path() == "/"
                    && parsed.query().is_none()
                    && parsed.fragment().is_none(),
                "CORS entries must be exact origins without credentials, paths, queries, or fragments"
            );
        }
        anyhow::ensure!(
            (1..=10_000).contains(&self.challenge_requests_per_minute_per_ip),
            "challenge_requests_per_minute_per_ip must be in 1..=10000"
        );
        for (name, value) in [
            (
                "abuse_reports_per_hour_per_ip",
                self.abuse_reports_per_hour_per_ip,
            ),
            (
                "abuse_reports_per_hour_per_key",
                self.abuse_reports_per_hour_per_key,
            ),
            (
                "abuse_reports_per_hour_per_target",
                self.abuse_reports_per_hour_per_target,
            ),
        ] {
            anyhow::ensure!((1..=1_000).contains(&value), "{name} must be in 1..=1000");
        }
        for network in &self.trusted_proxy_cidrs {
            network.parse::<ipnet::IpNet>().map_err(|error| {
                anyhow::anyhow!("invalid trusted proxy CIDR {network}: {error}")
            })?;
        }
        let mut profile_ids = std::collections::HashSet::new();
        anyhow::ensure!(
            self.admission_profiles.is_empty() || self.manifest_directory.is_some(),
            "manifest_directory is required when admission profiles are configured"
        );
        let mut semantic_build_identities = BTreeMap::new();
        for (public_digest, build) in &self.manifests.builds {
            build.public_document().validate()?;
            build.semantics().validate()?;
            anyhow::ensure!(
                build.public_digest() == *public_digest
                    && build.public_document().canonical_digest()? == *public_digest
                    && build.semantics().canonical_digest()? == build.semantic_digest(),
                "build registry entry {public_digest} does not match its immutable identities"
            );
            anyhow::ensure!(
                semantic_build_identities
                    .insert(build.semantic_digest(), *public_digest)
                    .is_none(),
                "multiple public build documents normalize to semantic build identity {}",
                build.semantic_digest()
            );
        }
        for (catalog_digest, catalog) in &self.manifests.campaign_content_manifests {
            for entry in &catalog.entries {
                let content = self
                    .manifests
                    .content_manifests
                    .get(&entry.content_manifest_sha256)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "campaign content catalog {catalog_digest} references a missing content manifest"
                        )
                    })?;
                anyhow::ensure!(
                    content.edition == catalog.edition && content.subject == entry.subject,
                    "campaign content catalog {catalog_digest} entry does not match its exact edition and subject"
                );
            }
        }
        for profile in &self.admission_profiles {
            anyhow::ensure!(
                profile_ids.insert(&profile.id),
                "duplicate admission profile ID: {}",
                profile.id
            );
            profile.content_subject.validate().map_err(|error| {
                anyhow::anyhow!(
                    "invalid content subject in admission profile {}: {error}",
                    profile.id
                )
            })?;
            OpaqueId::new(profile.template_id.clone()).map_err(|error| {
                anyhow::anyhow!(
                    "invalid template ID in admission profile {}: {error}",
                    profile.id
                )
            })?;
            anyhow::ensure!(
                !profile.allowed_scopes.is_empty()
                    && profile.allowed_scopes.iter().all(|scope| matches!(
                        scope.as_str(),
                        "individual_level" | "campaign_genesis" | "campaign_continuation"
                    )),
                "invalid allowed_scopes in admission profile {}",
                profile.id
            );
            anyhow::ensure!(
                !profile.allowed_metrics.is_empty()
                    && profile.allowed_metrics.iter().all(|metric| matches!(
                        metric.as_str(),
                        "original_score" | "fastest_success"
                    )),
                "invalid allowed_metrics in admission profile {}",
                profile.id
            );
            for (kind, value) in [
                ("build manifest", &profile.build_manifest_id),
                ("content manifest", &profile.content_manifest_id),
                ("config", &profile.config_id),
                ("ruleset", &profile.ruleset_id),
            ] {
                anyhow::ensure!(
                    value.len() == 64
                        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                        && value == &value.to_ascii_lowercase(),
                    "{kind} ID in profile {} must be 64 lowercase hexadecimal digits",
                    profile.id
                );
            }
            let build = digest32(&profile.build_manifest_id, "build manifest")?;
            let content = digest32(&profile.content_manifest_id, "content manifest")?;
            let rules_config = digest32(&profile.config_id, "rules config")?;
            let ruleset = digest32(&profile.ruleset_id, "ruleset manifest")?;
            let loaded_build = self.manifests.builds.get(&build).ok_or_else(|| {
                anyhow::anyhow!("profile {} references a missing build manifest", profile.id)
            })?;
            anyhow::ensure!(
                loaded_build.public_digest() == build,
                "profile {} resolved a build registry entry under the wrong public digest",
                profile.id
            );
            let build_doc = loaded_build.semantics();
            let content_doc = self
                .manifests
                .content_manifests
                .get(&content)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "profile {} references a missing content manifest",
                        profile.id
                    )
                })?;
            let rules_config_doc =
                self.manifests
                    .rules_configs
                    .get(&rules_config)
                    .ok_or_else(|| {
                        anyhow::anyhow!("profile {} references a missing rules config", profile.id)
                    })?;
            let published = self.manifests.rulesets.get(&ruleset).ok_or_else(|| {
                anyhow::anyhow!("profile {} references a missing ruleset", profile.id)
            })?;
            validate_current_sim_config(rules_config_doc).map_err(|error| {
                anyhow::anyhow!(
                    "profile {} references an invalid current SimConfig: {error}",
                    profile.id
                )
            })?;
            published
                .manifest
                .validate_ranked_simulation_policy(rules_config_doc)
                .map_err(|error| {
                    anyhow::anyhow!(
                        "profile {} ruleset simulation-policy identity mismatch: {error}",
                        profile.id
                    )
                })?;
            validate_current_ranked_build(loaded_build, published)?;
            let has_individual = profile
                .allowed_scopes
                .iter()
                .any(|scope| scope == "individual_level");
            let has_campaign_genesis = profile
                .allowed_scopes
                .iter()
                .any(|scope| scope == "campaign_genesis");
            let has_campaign_continuation = profile
                .allowed_scopes
                .iter()
                .any(|scope| scope == "campaign_continuation");
            let expected_scopes = expected_admission_scopes(
                profile.canonical_campaign_state.requirement.edition,
                &profile.content_subject,
            );
            anyhow::ensure!(
                profile
                    .allowed_scopes
                    .iter()
                    .map(String::as_str)
                    .eq(expected_scopes.iter().copied())
                    && (!has_individual
                        || published
                            .manifest
                            .board_scopes
                            .binary_search(&RulesetBoardScopeV1::IndividualLevel)
                            .is_ok())
                    && (!(has_campaign_genesis || has_campaign_continuation)
                        || (published
                            .manifest
                            .board_scopes
                            .binary_search(&RulesetBoardScopeV1::CampaignMission)
                            .is_ok()
                            && published
                                .manifest
                                .board_scopes
                                .binary_search(&RulesetBoardScopeV1::FullCampaign)
                                .is_ok())),
                "profile {} scopes do not match its exact edition/subject and immutable ruleset boards",
                profile.id
            );
            anyhow::ensure!(
                profile.allowed_metrics.iter().all(|metric| {
                    let metric = match metric.as_str() {
                        "original_score" => BoardMetricV1::OriginalScore,
                        "fastest_success" => BoardMetricV1::FastestSuccess,
                        _ => return false,
                    };
                    published.manifest.metrics.binary_search(&metric).is_ok()
                }),
                "profile {} metrics do not match its immutable ruleset",
                profile.id
            );
            anyhow::ensure!(
                published.manifest.rules_config_sha256 == rules_config
                    && published.manifest.canonical_campaign_state
                        == profile.canonical_campaign_state.requirement
                    && published
                        .manifest
                        .allowed_build_manifest_sha256
                        .binary_search(&build)
                        .is_ok()
                    && published
                        .manifest
                        .allowed_content_manifest_sha256
                        .binary_search(&content)
                        .is_ok()
                    && rules_config_doc.replay_schema_version == build_doc.replay_schema_version,
                "profile {} does not match its immutable manifest tuple",
                profile.id
            );
            profile
                .canonical_campaign_state
                .validate()
                .map_err(|error| {
                    anyhow::anyhow!(
                        "profile {} has an invalid canonical campaign-state pin: {error}",
                        profile.id
                    )
                })?;
            anyhow::ensure!(
                profile
                    .canonical_campaign_state
                    .requirement
                    .rules_config_sha256
                    == rules_config
                    && profile.canonical_campaign_state.requirement.edition == content_doc.edition,
                "profile {} campaign-state authority differs from its config or edition",
                profile.id
            );
            anyhow::ensure!(
                profile.ruleset_display_name == published.manifest.display_name
                    && profile.preset_id == published.manifest.preset_id.as_str()
                    && profile.preset_name == published.manifest.preset_name
                    && profile.difficulty_id == published.manifest.difficulty_id.as_str()
                    && profile.difficulty_name == published.manifest.difficulty_name,
                "profile {} labels do not match its digest-bound ruleset manifest",
                profile.id
            );
            anyhow::ensure!(
                content_doc.subject == profile.content_subject,
                "profile {} does not bind its exact typed content subject",
                profile.id
            );
            let offers_campaign = has_campaign_genesis || has_campaign_continuation;
            let campaign_content = profile
                .campaign_content_manifest_id
                .as_deref()
                .map(|value| digest32(value, "campaign content manifest"))
                .transpose()?;
            anyhow::ensure!(
                offers_campaign == campaign_content.is_some(),
                "profile {} must configure a campaign content manifest exactly for campaign scopes",
                profile.id
            );
            if let Some(campaign_content) = campaign_content {
                let catalog = self
                    .manifests
                    .campaign_content_manifests
                    .get(&campaign_content)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "profile {} campaign content manifest is missing",
                            profile.id
                        )
                    })?;
                anyhow::ensure!(
                    catalog.edition == content_doc.edition
                        && catalog.content_for(&content_doc.subject) == Some(content),
                    "profile {} campaign catalog does not contain its exact edition/subject content",
                    profile.id
                );
                anyhow::ensure!(
                    published
                        .manifest
                        .allowed_campaign_content_manifest_sha256
                        .binary_search(&campaign_content)
                        .is_ok(),
                    "profile {} campaign catalog is not allowlisted by its ruleset",
                    profile.id
                );
                published
                    .manifest
                    .validate_campaign_completion_catalog(catalog)
                    .map_err(|error| {
                        anyhow::anyhow!(
                            "profile {} campaign completion catalog is invalid: {error}",
                            profile.id
                        )
                    })?;
                anyhow::ensure!(
                    published.manifest.campaign_completion_policy.required()
                        == Some(&official_full_campaign_completion_policy_v1()),
                    "profile {} does not publish the exact official H12 completion predicate",
                    profile.id
                );
            }
            for policy in [
                &published.manifest.input_provenance_policy,
                &published.manifest.command_admission_policy,
                &published.manifest.submission_admission_policy,
                &published.manifest.verifier_policy,
            ] {
                let document = self
                    .manifests
                    .policies
                    .get(&policy.manifest_sha256)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "profile {} references a missing immutable policy",
                            profile.id
                        )
                    })?;
                anyhow::ensure!(
                    document.kind == policy.kind && document.version == policy.version,
                    "profile {} policy identity does not match its document",
                    profile.id
                );
            }
            anyhow::ensure!(
                profile.canonical_campaign_state.artifact.byte_length <= self.max_campaign_bytes,
                "canonical campaign state in profile {} exceeds the server limit",
                profile.id
            );
            anyhow::ensure!(
                profile.canonical_campaign_state_path.is_absolute(),
                "canonical campaign-state path in profile {} must be absolute",
                profile.id
            );
            let (actual_digest, actual_byte_length) =
                if let Some((authenticated_candidate_files, source_commit)) = candidate {
                    let expected_parent =
                        Path::new("/home/robinhood/.local/opt/robin-highscores/releases")
                            .join(source_commit)
                            .join("private/campaign-states");
                    hash_authenticated_candidate_campaign_state(
                        authenticated_candidate_files,
                        &profile.canonical_campaign_state_path,
                        &expected_parent,
                        HARD_MAX_CAMPAIGN_BYTES,
                    )?
                } else {
                    let path = &profile.canonical_campaign_state_path;
                    hash_regular_file_no_symlinks(path, HARD_MAX_CAMPAIGN_BYTES).map_err(
                        |error| {
                            anyhow::anyhow!(
                                "canonical campaign-state path in profile {} is unsafe: {error}",
                                profile.id
                            )
                        },
                    )?
                };
            anyhow::ensure!(
                actual_digest
                    == profile
                        .canonical_campaign_state
                        .artifact
                        .sha256
                        .into_bytes()
                    && actual_byte_length == profile.canonical_campaign_state.artifact.byte_length,
                "canonical campaign-state file in profile {} differs from its exact pin",
                profile.id
            );
            anyhow::ensure!(
                profile.viewer_available == profile.viewer_unavailable_reason.is_none(),
                "profile {} viewer availability and reason disagree",
                profile.id
            );
            anyhow::ensure!(
                (profile.viewer_available
                    && profile
                        .viewer_content_requirement
                        .is_some_and(|requirement| {
                            requirement.matches_edition(content_doc.edition)
                        }))
                    || (!profile.viewer_available && profile.viewer_content_requirement.is_none()),
                "profile {} viewer content requirement does not match its {:?} content edition",
                profile.id,
                content_doc.edition
            );
        }
        let mut competition_ids = std::collections::HashSet::new();
        for competition in &self.competitions {
            let manifest_sha256 = digest32(&competition.manifest_sha256, "competition manifest")?;
            let manifest = self
                .manifests
                .competitions
                .get(&manifest_sha256)
                .ok_or_else(|| anyhow::anyhow!("configured competition manifest is missing"))?;
            anyhow::ensure!(
                competition_ids.insert(manifest_sha256),
                "duplicate competition manifest: {}",
                competition.manifest_sha256
            );
            anyhow::ensure!(
                profile_ids.contains(&competition.admission_profile_id),
                "competition {} references an unknown profile",
                manifest.competition_id.as_str()
            );
            let profile = self
                .admission_profiles
                .iter()
                .find(|profile| profile.id == competition.admission_profile_id)
                .expect("profile membership checked above");
            let expected_content = match manifest.subject {
                robin_run_protocol::LeaderboardSubjectV1::Mission { .. } => {
                    robin_run_protocol::RunContentIdentityV1::Mission {
                        content_manifest_sha256: digest32(
                            &profile.content_manifest_id,
                            "content manifest",
                        )?,
                    }
                }
                robin_run_protocol::LeaderboardSubjectV1::FullCampaign => {
                    robin_run_protocol::RunContentIdentityV1::FullCampaign {
                        campaign_content_manifest_sha256: digest32(
                            profile
                                .campaign_content_manifest_id
                                .as_deref()
                                .ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "full-campaign competition profile has no campaign catalog"
                                    )
                                })?,
                            "campaign content manifest",
                        )?,
                    }
                }
            };
            anyhow::ensure!(
                manifest.content == expected_content
                    && manifest.rules_config_sha256
                        == digest32(&profile.config_id, "rules config")?
                    && manifest.ruleset_manifest_sha256
                        == digest32(&profile.ruleset_id, "ruleset manifest")?
                    && manifest.canonical_campaign_state
                        == profile.canonical_campaign_state.requirement,
                "competition {} does not match its admission profile tuple",
                manifest.competition_id.as_str()
            );
            anyhow::ensure!(
                profile.allowed_metrics.iter().any(|metric| {
                    matches!(
                        (metric.as_str(), manifest.metric),
                        ("original_score", BoardMetricV1::OriginalScore)
                            | ("fastest_success", BoardMetricV1::FastestSuccess)
                    )
                }),
                "competition {} metric is not enabled by its admission profile",
                manifest.competition_id.as_str()
            );
        }
        Ok(())
    }

    pub fn load_or_create_cursor_key(&self) -> anyhow::Result<[u8; 32]> {
        private_key(&self.cursor_secret_path, true, "cursor secret")
    }

    /// Load the already initialized pagination key. Serving processes must
    /// fail closed rather than silently replacing a lost durable identity.
    pub fn load_cursor_key(&self) -> anyhow::Result<[u8; 32]> {
        private_key(&self.cursor_secret_path, false, "cursor secret")
    }

    pub fn load_or_create_competition_run_grant_key(&self) -> anyhow::Result<[u8; 32]> {
        private_key(
            &self.competition_run_grant_secret_path,
            true,
            "competition run grant secret",
        )
    }

    pub fn load_competition_run_grant_key(&self) -> anyhow::Result<[u8; 32]> {
        private_key(
            &self.competition_run_grant_secret_path,
            false,
            "competition run grant secret",
        )
    }

    pub fn load_or_create_run_preflight_grant_key(&self) -> anyhow::Result<[u8; 32]> {
        private_key(
            &self.run_preflight_grant_secret_path,
            true,
            "run preflight grant secret",
        )
    }

    pub fn load_run_preflight_grant_key(&self) -> anyhow::Result<[u8; 32]> {
        private_key(
            &self.run_preflight_grant_secret_path,
            false,
            "run preflight grant secret",
        )
    }

    /// Exercise the legacy create-new primitive only in its focused unit
    /// tests. Production initialization is transaction-bound in the admin
    /// binary and this lower-level, non-resumable path must not be callable.
    #[cfg(test)]
    pub(crate) fn initialize_backup_authority_hmac_key(&self) -> anyhow::Result<[u8; 32]> {
        backup_authority_key(
            &self.backup_authority_hmac_secret_path,
            true,
            "backup authority HMAC secret",
        )
    }

    /// Load the already initialized backup authority without creating or
    /// repairing it. Every inode property is checked on the pinned descriptor.
    pub fn load_backup_authority_hmac_key(&self) -> anyhow::Result<[u8; 32]> {
        backup_authority_key(
            &self.backup_authority_hmac_secret_path,
            false,
            "backup authority HMAC secret",
        )
    }
}

fn validate_current_ranked_build(
    build: &LoadedBuildManifest,
    published: &PublishedRulesetV1,
) -> anyhow::Result<()> {
    if matches!(
        published.operational_status,
        RulesetOperationalStatusV1::Active
    ) && published.manifest.replay_schema_versions
        == [robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1]
    {
        anyhow::ensure!(
            matches!(build.public_document(), VersionedBuildManifest::V2(_)),
            "active current-schema rulesets require a public BuildManifestV2"
        );
        anyhow::ensure!(
            build.semantics().save_schema_version
                == robin_run_protocol::CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1,
            "active current-schema rulesets require current save schema {}",
            robin_run_protocol::CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1
        );
        anyhow::ensure!(
            build.semantics().network_protocol_version
                == robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1
                && published.manifest.network_protocol_versions
                    == [robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1],
            "active current-schema rulesets require exactly network protocol {}",
            robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1
        );
    }
    Ok(())
}

/// Prove that an operator rules document contains exactly the current engine's
/// complete deterministic configuration: serde defaults and unknown fields
/// are not allowed to silently normalize a board identity.
fn validate_current_sim_config(rules: &RulesConfigIdentityV1) -> anyhow::Result<()> {
    rules.validate()?;
    anyhow::ensure!(
        rules.replay_schema_version == robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        "rules config replay schema is not current"
    );
    let input = CanonicalValue::Object(rules.sim_config.clone());
    let config: robin_engine::engine::SimConfig = serde_json::from_value(
        serde_json::to_value(&input).context("serialize canonical SimConfig")?,
    )
    .context("decode current SimConfig")?;
    let canonical =
        CanonicalValue::from_serializable(&config).context("canonicalize current SimConfig")?;
    anyhow::ensure!(
        canonical == input,
        "SimConfig contains missing, unknown, or default-normalized fields"
    );
    Ok(())
}

impl ManifestRegistry {
    fn load(root: &Path) -> anyhow::Result<Self> {
        anyhow::ensure!(root.is_absolute(), "manifest_directory must be absolute");
        let metadata = std::fs::symlink_metadata(root)?;
        anyhow::ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "manifest_directory must be a non-symlink directory"
        );
        Ok(Self {
            builds: load_build_documents(root)?,
            content_manifests: load_documents(
                root,
                "content-manifests",
                |document: &ContentManifestV1| {
                    document.validate()?;
                    Ok(document.canonical_digest()?)
                },
            )?,
            campaign_content_manifests: load_documents(
                root,
                "campaign-content-manifests",
                |document: &CampaignContentManifestV1| {
                    document.validate()?;
                    Ok(document.canonical_digest()?)
                },
            )?,
            rules_configs: load_documents(
                root,
                "rules-configs",
                |document: &RulesConfigIdentityV1| {
                    document.validate()?;
                    Ok(document.canonical_digest()?)
                },
            )?,
            rulesets: load_published_rulesets(root)?,
            competitions: load_documents(
                root,
                "competitions",
                |document: &CompetitionManifestV1| {
                    document.validate()?;
                    Ok(document.canonical_digest()?)
                },
            )?,
            policies: load_documents(root, "policies", |document: &ImmutablePolicyManifestV1| {
                document.validate()?;
                Ok(document.canonical_digest()?)
            })?,
        })
    }

    /// Load the immutable manifest registry from bytes captured by an already
    /// authenticated candidate scan. This is intentionally crate-private:
    /// serving uses installed paths, while deployment probes must never reopen
    /// a candidate pathname after its descriptors were authenticated.
    pub(crate) fn load_from_candidate(
        authenticated_candidate_files: &BTreeMap<String, Vec<u8>>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            builds: load_candidate_build_documents(authenticated_candidate_files)?,
            content_manifests: load_candidate_documents(
                authenticated_candidate_files,
                "content-manifests",
                |document: &ContentManifestV1| {
                    document.validate()?;
                    Ok(document.canonical_digest()?)
                },
            )?,
            campaign_content_manifests: load_candidate_documents(
                authenticated_candidate_files,
                "campaign-content-manifests",
                |document: &CampaignContentManifestV1| {
                    document.validate()?;
                    Ok(document.canonical_digest()?)
                },
            )?,
            rules_configs: load_candidate_documents(
                authenticated_candidate_files,
                "rules-configs",
                |document: &RulesConfigIdentityV1| {
                    document.validate()?;
                    Ok(document.canonical_digest()?)
                },
            )?,
            rulesets: load_candidate_published_rulesets(authenticated_candidate_files)?,
            competitions: load_candidate_documents(
                authenticated_candidate_files,
                "competitions",
                |document: &CompetitionManifestV1| {
                    document.validate()?;
                    Ok(document.canonical_digest()?)
                },
            )?,
            policies: load_candidate_documents(
                authenticated_candidate_files,
                "policies",
                |document: &ImmutablePolicyManifestV1| {
                    document.validate()?;
                    Ok(document.canonical_digest()?)
                },
            )?,
        })
    }
}

fn load_candidate_build_documents(
    authenticated_candidate_files: &BTreeMap<String, Vec<u8>>,
) -> anyhow::Result<BTreeMap<Digest32, LoadedBuildManifest>> {
    let public_documents = load_candidate_documents(
        authenticated_candidate_files,
        "builds",
        |document: &VersionedBuildManifest| {
            document.validate()?;
            Ok(document.canonical_digest()?)
        },
    )?;
    let mut builds = BTreeMap::new();
    let mut semantic_identities = BTreeMap::new();
    for (public_digest, public_document) in public_documents {
        let loaded = LoadedBuildManifest::new(public_document)?;
        anyhow::ensure!(
            loaded.public_digest() == public_digest,
            "loaded candidate build document changed public identity"
        );
        anyhow::ensure!(
            semantic_identities
                .insert(loaded.semantic_digest(), public_digest)
                .is_none(),
            "multiple candidate build documents normalize to semantic build identity {}",
            loaded.semantic_digest()
        );
        anyhow::ensure!(
            builds.insert(public_digest, loaded).is_none(),
            "duplicate candidate build manifest digest {public_digest}"
        );
    }
    Ok(builds)
}

fn load_candidate_published_rulesets(
    authenticated_candidate_files: &BTreeMap<String, Vec<u8>>,
) -> anyhow::Result<BTreeMap<Digest32, PublishedRulesetV1>> {
    let immutable = load_candidate_documents(
        authenticated_candidate_files,
        "ruleset-manifests",
        |document: &RulesetManifestV1| {
            document.validate()?;
            Ok(document.canonical_digest()?)
        },
    )?;
    let published = load_candidate_documents(
        authenticated_candidate_files,
        "published-rulesets",
        |document: &PublishedRulesetV1| {
            document.validate()?;
            Ok(document.ruleset_manifest_sha256)
        },
    )?;
    anyhow::ensure!(
        immutable.len() == published.len(),
        "candidate ruleset manifest and publication directories have different identity sets"
    );
    for (digest, publication) in &published {
        anyhow::ensure!(
            immutable.get(digest) == Some(&publication.manifest),
            "candidate published ruleset {digest} embeds a substituted manifest"
        );
    }
    anyhow::ensure!(
        immutable
            .keys()
            .all(|digest| published.contains_key(digest)),
        "candidate immutable ruleset has no publication status"
    );
    Ok(published)
}

fn load_candidate_documents<T, F>(
    authenticated_candidate_files: &BTreeMap<String, Vec<u8>>,
    kind: &str,
    mut identity: F,
) -> anyhow::Result<BTreeMap<Digest32, T>>
where
    T: DeserializeOwned,
    F: FnMut(&T) -> anyhow::Result<Digest32>,
{
    let prefix = format!("config/manifests/{kind}/");
    let mut documents = BTreeMap::new();
    for (path, bytes) in authenticated_candidate_files {
        let Some(name) = path.strip_prefix(&prefix) else {
            continue;
        };
        anyhow::ensure!(
            !name.is_empty() && !name.contains('/'),
            "candidate manifest registry contains a non-file entry"
        );
        anyhow::ensure!(
            documents.len() < HARD_MAX_CANDIDATE_MANIFEST_DOCUMENTS,
            "candidate manifest directory exceeds its document-count limit"
        );
        let stem = name
            .strip_suffix(".json")
            .ok_or_else(|| anyhow::anyhow!("candidate manifest filename must end in .json"))?;
        let expected = digest32(stem, "candidate manifest filename")?;
        anyhow::ensure!(
            bytes.len() <= usize::try_from(HARD_MAX_OPERATOR_DOCUMENT_BYTES)?,
            "candidate manifest document exceeds its byte limit"
        );
        let document: T = serde_json::from_slice(bytes)?;
        let actual = identity(&document)?;
        anyhow::ensure!(
            actual == expected,
            "candidate manifest document digest does not match filename {name}"
        );
        anyhow::ensure!(
            documents.insert(actual, document).is_none(),
            "duplicate candidate manifest digest {actual}"
        );
    }
    Ok(documents)
}

fn load_build_documents(root: &Path) -> anyhow::Result<BTreeMap<Digest32, LoadedBuildManifest>> {
    let public_documents = load_documents(root, "builds", |document: &VersionedBuildManifest| {
        document.validate()?;
        Ok(document.canonical_digest()?)
    })?;
    let mut builds = BTreeMap::new();
    let mut semantic_identities = BTreeMap::new();
    for (public_digest, public_document) in public_documents {
        let loaded = LoadedBuildManifest::new(public_document)?;
        anyhow::ensure!(
            loaded.public_digest() == public_digest,
            "loaded build document changed public identity"
        );
        anyhow::ensure!(
            semantic_identities
                .insert(loaded.semantic_digest(), public_digest)
                .is_none(),
            "multiple public build documents normalize to semantic build identity {}",
            loaded.semantic_digest()
        );
        anyhow::ensure!(
            builds.insert(public_digest, loaded).is_none(),
            "duplicate public build manifest digest {public_digest}"
        );
    }
    Ok(builds)
}

/// Load the exact split emitted by `robin_manifest_tool`: immutable ruleset
/// semantics are digest-addressed separately from their mutable
/// publication/quarantine wrappers. Serving never accepts an embedded
/// replacement manifest merely because the wrapper names the expected digest.
fn load_published_rulesets(root: &Path) -> anyhow::Result<BTreeMap<Digest32, PublishedRulesetV1>> {
    let immutable = load_documents(root, "ruleset-manifests", |document: &RulesetManifestV1| {
        document.validate()?;
        Ok(document.canonical_digest()?)
    })?;
    let published = load_documents(
        root,
        "published-rulesets",
        |document: &PublishedRulesetV1| {
            document.validate()?;
            Ok(document.ruleset_manifest_sha256)
        },
    )?;
    anyhow::ensure!(
        immutable.len() == published.len(),
        "ruleset manifest and publication directories have different identity sets"
    );
    for (digest, publication) in &published {
        let manifest = immutable.get(digest).ok_or_else(|| {
            anyhow::anyhow!("published ruleset {digest} has no immutable manifest")
        })?;
        anyhow::ensure!(
            &publication.manifest == manifest,
            "published ruleset {digest} embeds a manifest that differs from the immutable document"
        );
    }
    for digest in immutable.keys() {
        anyhow::ensure!(
            published.contains_key(digest),
            "immutable ruleset {digest} has no publication status"
        );
    }
    Ok(published)
}

fn load_documents<T, F>(
    root: &Path,
    kind: &str,
    mut identity: F,
) -> anyhow::Result<BTreeMap<Digest32, T>>
where
    T: DeserializeOwned,
    F: FnMut(&T) -> anyhow::Result<Digest32>,
{
    let directory = root.join(kind);
    let metadata = std::fs::symlink_metadata(&directory)?;
    anyhow::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "{kind} manifest directory must be a non-symlink directory"
    );
    let mut documents = BTreeMap::new();
    for entry in std::fs::read_dir(&directory)? {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        anyhow::ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "manifest entries must be regular non-symlink files"
        );
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("manifest filename is not UTF-8"))?;
        let stem = name
            .strip_suffix(".json")
            .ok_or_else(|| anyhow::anyhow!("manifest filename must end in .json"))?;
        let expected = digest32(stem, "manifest filename")?;
        let bytes = read_regular_file_no_symlinks(&entry.path(), HARD_MAX_OPERATOR_DOCUMENT_BYTES)?;
        let document: T = serde_json::from_slice(&bytes)?;
        let actual = identity(&document)?;
        anyhow::ensure!(
            actual == expected,
            "manifest document digest does not match filename {name}"
        );
        anyhow::ensure!(
            documents.insert(actual, document).is_none(),
            "duplicate manifest digest {actual}"
        );
    }
    Ok(documents)
}

#[cfg(target_os = "linux")]
fn open_regular_no_symlinks(path: &Path) -> anyhow::Result<std::fs::File> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    let fd = openat2(
        rustix::fs::CWD,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let file = std::fs::File::from(fd);
    anyhow::ensure!(file.metadata()?.is_file(), "path is not a regular file");
    Ok(file)
}

#[cfg(not(target_os = "linux"))]
fn open_regular_no_symlinks(path: &Path) -> anyhow::Result<std::fs::File> {
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "path is not a regular non-symlink file"
    );
    let file = std::fs::File::open(path)?;
    anyhow::ensure!(file.metadata()?.is_file(), "path is not a regular file");
    Ok(file)
}

fn read_regular_file_no_symlinks(path: &Path, limit: u64) -> anyhow::Result<Vec<u8>> {
    let file = open_regular_no_symlinks(path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.len() <= limit,
        "operator document exceeds the {limit}-byte safety limit"
    );
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len())?);
    file.take(limit + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= limit,
        "operator document grew beyond its limit"
    );
    Ok(bytes)
}

fn hash_authenticated_candidate_campaign_state(
    authenticated_candidate_files: &BTreeMap<String, Vec<u8>>,
    configured_absolute_path: &Path,
    expected_ambient_parent: &Path,
    limit: u64,
) -> anyhow::Result<([u8; 32], u64)> {
    anyhow::ensure!(
        configured_absolute_path.is_absolute()
            && configured_absolute_path.parent() == Some(expected_ambient_parent),
        "canonical campaign-state path escapes the candidate release"
    );
    let name = configured_absolute_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("canonical campaign state has no filename"))?;
    let relative = Path::new("private/campaign-states").join(name);
    let relative = relative
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("canonical campaign-state filename is not UTF-8"))?;
    let bytes = authenticated_candidate_files
        .get(relative)
        .ok_or_else(|| anyhow::anyhow!("authenticated candidate omits campaign state"))?;
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() <= usize::try_from(limit)?,
        "authenticated candidate campaign state is empty or exceeds its byte limit"
    );
    Ok((Sha256::digest(bytes).into(), u64::try_from(bytes.len())?))
}

fn hash_regular_file_no_symlinks(path: &Path, limit: u64) -> anyhow::Result<([u8; 32], u64)> {
    let mut file = open_regular_no_symlinks(path)?;
    let initial_length = file.metadata()?.len();
    anyhow::ensure!(
        initial_length > 0 && initial_length <= limit,
        "operator file is empty or exceeds its byte limit"
    );
    let mut hasher = Sha256::new();
    let mut byte_length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        byte_length = byte_length
            .checked_add(u64::try_from(count)?)
            .ok_or_else(|| anyhow::anyhow!("operator file size overflow"))?;
        anyhow::ensure!(
            byte_length <= limit,
            "operator file grew beyond its byte limit"
        );
        hasher.update(&buffer[..count]);
    }
    anyhow::ensure!(
        byte_length == initial_length,
        "operator file changed length while hashing"
    );
    Ok((hasher.finalize().into(), byte_length))
}

fn digest32(value: &str, field: &str) -> anyhow::Result<Digest32> {
    anyhow::ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "{field} must be 64 lowercase hexadecimal digits"
    );
    let bytes = hex::decode(value)?;
    Ok(Digest32::from_bytes(bytes.try_into().map_err(|_| {
        anyhow::anyhow!("{field} must be 32 bytes")
    })?))
}

#[cfg(target_os = "linux")]
fn load_private_bearer_token(path: &Path) -> anyhow::Result<Vec<u8>> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::io::Read as _;
    use std::os::unix::fs::PermissionsExt as _;

    anyhow::ensure!(
        path.is_absolute(),
        "moderation bearer token path must be absolute"
    );
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("moderation bearer token must have a parent directory"))?;
    let filename = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("moderation bearer token must name a file"))?;
    let parent_fd = openat2(
        rustix::fs::CWD,
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|error| {
        anyhow::anyhow!(
            "moderation bearer token parent must pre-exist without symlinks ({}): {error}",
            parent.display()
        )
    })?;
    let parent_file = std::fs::File::from(parent_fd);
    let token_fd = openat2(
        &parent_file,
        filename,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let file = std::fs::File::from(token_fd);
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file(),
        "moderation bearer token must be a regular file"
    );
    anyhow::ensure!(
        metadata.permissions().mode() & 0o777 == 0o400,
        "moderation bearer token permissions must be exactly 0400"
    );
    anyhow::ensure!(
        (32..=128).contains(&metadata.len()),
        "moderation bearer token must contain 32..=128 bytes"
    );
    let mut token = Vec::with_capacity(metadata.len() as usize);
    file.take(129).read_to_end(&mut token)?;
    anyhow::ensure!(
        (32..=128).contains(&token.len()) && token.iter().all(|byte| byte.is_ascii_graphic()),
        "moderation bearer token must contain 32..=128 printable ASCII bytes without whitespace"
    );
    Ok(token)
}

#[cfg(not(target_os = "linux"))]
fn load_private_bearer_token(_path: &Path) -> anyhow::Result<Vec<u8>> {
    anyhow::bail!("pinned moderation bearer token loading requires Linux openat2 confinement")
}

#[cfg(target_os = "linux")]
fn private_key(path: &Path, create_if_missing: bool, label: &str) -> anyhow::Result<[u8; 32]> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::PermissionsExt as _;

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{label} must have a parent directory"))?;
    let filename = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{label} must name a file"))?;
    let parent_fd = openat2(
        rustix::fs::CWD,
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|error| {
        anyhow::anyhow!(
            "{label} parent must pre-exist without symlinks ({}): {error}",
            parent.display()
        )
    })?;
    let parent_file = std::fs::File::from(parent_fd);
    let parent_metadata = parent_file.metadata()?;
    anyhow::ensure!(
        parent_metadata.is_dir() && parent_metadata.permissions().mode() & 0o077 == 0,
        "{label} parent must be a private directory (0700 or stricter)"
    );

    let open_existing = || -> anyhow::Result<std::fs::File> {
        let fd = openat2(
            &parent_file,
            filename,
            OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )?;
        Ok(std::fs::File::from(fd))
    };

    let mut file = if create_if_missing {
        match openat2(
            &parent_file,
            filename,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o400),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        ) {
            Ok(fd) => {
                let mut file = std::fs::File::from(fd);
                let key: [u8; 32] = rand::random();
                file.write_all(&key)?;
                file.sync_all()?;
                parent_file.sync_all()?;
                return Ok(key);
            }
            Err(rustix::io::Errno::EXIST) => open_existing()?,
            Err(error) => return Err(error.into()),
        }
    } else {
        open_existing().map_err(|error| {
            anyhow::anyhow!(
                "{label} is not initialized; run the matching robin-highscores-admin initialize command: {error}"
            )
        })?
    };
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.len() == 32
            && metadata.permissions().mode() & 0o777 == 0o400,
        "{label} must be an exact 32-byte private regular file mode 0400"
    );
    let mut key = [0; 32];
    file.read_exact(&mut key)?;
    let mut trailing = [0; 1];
    anyhow::ensure!(file.read(&mut trailing)? == 0, "{label} must be 32 bytes");
    Ok(key)
}

#[cfg(target_os = "linux")]
fn backup_authority_key(path: &Path, create_new: bool, label: &str) -> anyhow::Result<[u8; 32]> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{label} must have a parent directory"))?;
    let filename = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{label} must name a file"))?;
    let parent_fd = openat2(
        rustix::fs::CWD,
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|error| {
        anyhow::anyhow!(
            "{label} parent must pre-exist without symlinks ({}): {error}",
            parent.display()
        )
    })?;
    let parent_file = std::fs::File::from(parent_fd);
    let parent_metadata = parent_file.metadata()?;
    anyhow::ensure!(
        parent_metadata.is_dir()
            && parent_metadata.permissions().mode() & 0o777 == 0o700
            && parent_metadata.uid() == rustix::process::geteuid().as_raw(),
        "{label} parent must be an effective-user-owned directory mode 0700"
    );

    let flags = if create_new {
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC
    } else {
        OFlags::RDONLY | OFlags::CLOEXEC
    };
    let fd = openat2(
        &parent_file,
        filename,
        flags,
        if create_new {
            Mode::from_raw_mode(0o400)
        } else {
            Mode::empty()
        },
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|error| {
        if create_new {
            anyhow::anyhow!(
                "{label} initialization requires an absent destination ({}): {error}",
                path.display()
            )
        } else {
            anyhow::anyhow!(
                "{label} is not initialized; use the transaction-bound VPS backup-authority initializer: {error}"
            )
        }
    })?;
    let mut file = std::fs::File::from(fd);

    if create_new {
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.is_file()
                && metadata.len() == 0
                && metadata.permissions().mode() & 0o777 == 0o400
                && metadata.nlink() == 1
                && metadata.uid() == rustix::process::geteuid().as_raw(),
            "new {label} must be a private regular file mode 0400 with exactly one hard link"
        );
        let key = loop {
            let candidate: [u8; 32] = rand::random();
            if candidate != [0; 32] {
                break candidate;
            }
        };
        file.write_all(&key)?;
        file.sync_all()?;
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.is_file()
                && metadata.len() == 32
                && metadata.permissions().mode() & 0o777 == 0o400
                && metadata.nlink() == 1
                && metadata.uid() == rustix::process::geteuid().as_raw(),
            "new {label} changed identity or permissions during initialization"
        );
        parent_file.sync_all()?;
        revalidate_backup_authority_path(
            parent,
            filename,
            &parent_metadata,
            &metadata,
            &key,
            label,
        )?;
        return Ok(key);
    }

    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.len() == 32
            && metadata.permissions().mode() & 0o777 == 0o400
            && metadata.nlink() == 1
            && metadata.uid() == rustix::process::geteuid().as_raw(),
        "{label} must be an exact 32-byte private regular file mode 0400 with exactly one hard link"
    );
    let mut key = [0; 32];
    file.read_exact(&mut key)?;
    let mut trailing = [0; 1];
    anyhow::ensure!(file.read(&mut trailing)? == 0, "{label} must be 32 bytes");
    anyhow::ensure!(key != [0; 32], "{label} must not be the all-zero key");
    revalidate_backup_authority_path(parent, filename, &parent_metadata, &metadata, &key, label)?;
    Ok(key)
}

#[cfg(target_os = "linux")]
fn revalidate_backup_authority_path(
    parent: &Path,
    filename: &std::ffi::OsStr,
    expected_parent: &std::fs::Metadata,
    expected_file: &std::fs::Metadata,
    expected_key: &[u8; 32],
    label: &str,
) -> anyhow::Result<()> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::io::Read as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let parent_fd = openat2(
        rustix::fs::CWD,
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let parent_file = std::fs::File::from(parent_fd);
    let parent_metadata = parent_file.metadata()?;
    anyhow::ensure!(
        parent_metadata.is_dir()
            && parent_metadata.dev() == expected_parent.dev()
            && parent_metadata.ino() == expected_parent.ino()
            && parent_metadata.permissions().mode() & 0o777 == 0o700
            && parent_metadata.uid() == rustix::process::geteuid().as_raw(),
        "{label} parent path changed during access"
    );
    let file_fd = openat2(
        &parent_file,
        filename,
        OFlags::RDONLY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut file = std::fs::File::from(file_fd);
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file()
            && metadata.dev() == expected_file.dev()
            && metadata.ino() == expected_file.ino()
            && metadata.len() == 32
            && metadata.permissions().mode() & 0o777 == 0o400
            && metadata.nlink() == 1
            && metadata.uid() == rustix::process::geteuid().as_raw(),
        "{label} path changed during access"
    );
    let mut key = [0; 32];
    file.read_exact(&mut key)?;
    let mut trailing = [0; 1];
    anyhow::ensure!(
        file.read(&mut trailing)? == 0 && &key == expected_key,
        "{label} contents changed during access"
    );
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn private_key(_path: &Path, _create_if_missing: bool, label: &str) -> anyhow::Result<[u8; 32]> {
    anyhow::bail!("{label} access requires Linux openat2 confinement")
}

#[cfg(not(target_os = "linux"))]
fn backup_authority_key(_path: &Path, _create_new: bool, label: &str) -> anyhow::Result<[u8; 32]> {
    anyhow::bail!("{label} access requires Linux openat2 confinement")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_manifest_registry_directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        for kind in [
            "builds",
            "content-manifests",
            "campaign-content-manifests",
            "rules-configs",
            "ruleset-manifests",
            "published-rulesets",
            "competitions",
            "policies",
        ] {
            std::fs::create_dir(directory.path().join(kind)).unwrap();
        }
        directory
    }

    #[test]
    fn viewer_content_requirements_are_exactly_bound_to_official_edition() {
        assert!(
            ViewerContentRequirementConfig::BundledDemo
                .matches_edition(OfficialContentEditionV1::Demo)
        );
        assert!(
            ViewerContentRequirementConfig::UserLocalRetail
                .matches_edition(OfficialContentEditionV1::Full)
        );
        assert!(
            !ViewerContentRequirementConfig::BundledDemo
                .matches_edition(OfficialContentEditionV1::Full)
        );
        assert!(
            !ViewerContentRequirementConfig::UserLocalRetail
                .matches_edition(OfficialContentEditionV1::Demo)
        );
    }

    #[test]
    fn admission_scopes_bind_genesis_only_to_authentic_full_campaign_first_mission() {
        let full_genesis = OfficialContentSubjectV1::FieldMission {
            mission_id: OFFICIAL_FULL_CAMPAIGN_GENESIS_MISSION_ID_V1.to_owned(),
        };
        assert_eq!(
            expected_admission_scopes(OfficialContentEditionV1::Full, &full_genesis),
            ["campaign_genesis", "campaign_continuation"]
        );

        let later_field = OfficialContentSubjectV1::FieldMission {
            mission_id: "H12_Not_MP".to_owned(),
        };
        assert_eq!(
            expected_admission_scopes(OfficialContentEditionV1::Full, &later_field),
            ["campaign_continuation"]
        );

        let headquarters = OfficialContentSubjectV1::Headquarters {
            mission_id: robin_run_protocol::OFFICIAL_FULL_HEADQUARTERS_MISSION_ID_V1.to_owned(),
        };
        assert_eq!(
            expected_admission_scopes(OfficialContentEditionV1::Full, &headquarters),
            ["campaign_continuation"]
        );
        assert_eq!(
            expected_admission_scopes(OfficialContentEditionV1::Demo, &full_genesis),
            ["individual_level"]
        );
    }

    fn write_build_document(directory: &Path, digest: Digest32, document: &VersionedBuildManifest) {
        std::fs::write(
            directory.join("builds").join(format!("{digest}.json")),
            document.canonical_bytes().unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn unsafe_limits_and_origins_are_rejected() {
        let config = ServerConfig {
            max_replay_bytes: HARD_MAX_REPLAY_BYTES + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());

        let config = ServerConfig {
            max_concurrent_requests: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());

        let mut config = ServerConfig::default();
        config.upload_reservation_ttl_seconds = config.upload_timeout_seconds + 29;
        assert!(config.validate().is_err());

        let config = ServerConfig {
            upload_reservation_ttl_seconds: 24 * 60 * 60 + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());

        let config = ServerConfig {
            minimum_storage_free_bytes: crate::storage_admission::MINIMUM_STORAGE_RESERVE_BYTES - 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());

        let config = ServerConfig {
            allowed_origins: vec!["http://example.com".to_owned()],
            ..Default::default()
        };
        assert!(config.validate().is_err());

        let config = ServerConfig {
            allowed_origins: vec!["http://localhost.evil.example".to_owned()],
            ..Default::default()
        };
        assert!(config.validate().is_err());

        let config = ServerConfig {
            allowed_origins: vec!["http://127.0.0.1:3000".to_owned()],
            ..Default::default()
        };
        assert!(config.validate().is_ok());

        let config = ServerConfig {
            allowed_origins: vec!["http://127.0.0.2:3000".to_owned()],
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn cursor_secret_is_durable_and_exact_length() {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let config = ServerConfig {
            cursor_secret_path: directory.path().join("cursor.key"),
            competition_run_grant_secret_path: directory.path().join("grant.key"),
            run_preflight_grant_secret_path: directory.path().join("preflight.key"),
            ..Default::default()
        };
        assert!(config.load_cursor_key().is_err());
        let first = config.load_or_create_cursor_key().unwrap();
        assert_eq!(config.load_cursor_key().unwrap(), first);
        let second = config.load_or_create_cursor_key().unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read(&config.cursor_secret_path).unwrap().len(), 32);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&config.cursor_secret_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o400
            );
        }
        let grant = config.load_or_create_competition_run_grant_key().unwrap();
        assert_eq!(config.load_competition_run_grant_key().unwrap(), grant);
        assert_ne!(grant, first);
        let preflight = config.load_or_create_run_preflight_grant_key().unwrap();
        assert_eq!(config.load_run_preflight_grant_key().unwrap(), preflight);
        assert_ne!(preflight, first);
        assert_ne!(preflight, grant);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cursor_secret_rejects_a_symlink_without_following_it() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let target = directory.path().join("target.key");
        std::fs::write(&target, [7_u8; 32]).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o400)).unwrap();
        let link = directory.path().join("cursor.key");
        symlink(&target, &link).unwrap();

        let config = ServerConfig {
            cursor_secret_path: link,
            ..Default::default()
        };
        assert!(config.load_or_create_cursor_key().is_err());
        assert_eq!(std::fs::read(target).unwrap(), [7_u8; 32]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn backup_authority_is_create_new_and_rejects_aliases() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};

        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let authority = directory.path().join("backup-authority.key");
        let mut config = ServerConfig {
            backup_authority_hmac_secret_path: authority.clone(),
            ..Default::default()
        };

        assert!(config.load_backup_authority_hmac_key().is_err());
        let initialized = config.initialize_backup_authority_hmac_key().unwrap();
        assert_eq!(
            config.load_backup_authority_hmac_key().unwrap(),
            initialized
        );
        assert!(config.initialize_backup_authority_hmac_key().is_err());

        let metadata = std::fs::metadata(&authority).unwrap();
        assert_eq!(metadata.len(), 32);
        assert_eq!(metadata.permissions().mode() & 0o777, 0o400);
        assert_eq!(metadata.nlink(), 1);
        assert_eq!(metadata.uid(), rustix::process::geteuid().as_raw());

        std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&authority, [0_u8; 32]).unwrap();
        std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o400)).unwrap();
        assert!(config.load_backup_authority_hmac_key().is_err());
        std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&authority, initialized).unwrap();
        std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o400)).unwrap();

        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        assert!(config.load_backup_authority_hmac_key().is_err());
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            config.load_backup_authority_hmac_key().unwrap(),
            initialized
        );

        let hardlink = directory.path().join("backup-authority-hardlink.key");
        std::fs::hard_link(&authority, &hardlink).unwrap();
        assert!(config.load_backup_authority_hmac_key().is_err());
        std::fs::remove_file(&hardlink).unwrap();
        assert_eq!(
            config.load_backup_authority_hmac_key().unwrap(),
            initialized
        );

        let symlink_path = directory.path().join("backup-authority-symlink.key");
        symlink(&authority, &symlink_path).unwrap();
        config.backup_authority_hmac_secret_path = symlink_path;
        assert!(config.load_backup_authority_hmac_key().is_err());

        let mut duplicate = ServerConfig::default();
        duplicate.backup_authority_hmac_secret_path = duplicate.cursor_secret_path.clone();
        assert!(duplicate.validate().is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn operator_files_are_hashed_from_pinned_non_symlink_inodes() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("campaign.bin");
        std::fs::write(&file, b"canonical campaign").unwrap();
        assert_eq!(
            hash_regular_file_no_symlinks(&file, HARD_MAX_CAMPAIGN_BYTES).unwrap(),
            (
                Sha256::digest(b"canonical campaign").into(),
                b"canonical campaign".len() as u64,
            )
        );

        let link = directory.path().join("campaign-link.bin");
        symlink(&file, &link).unwrap();
        assert!(hash_regular_file_no_symlinks(&link, HARD_MAX_CAMPAIGN_BYTES).is_err());

        let real_parent = directory.path().join("real-parent");
        std::fs::create_dir(&real_parent).unwrap();
        std::fs::write(real_parent.join("campaign.bin"), b"canonical campaign").unwrap();
        let parent_link = directory.path().join("parent-link");
        symlink(&real_parent, &parent_link).unwrap();
        assert!(
            hash_regular_file_no_symlinks(
                &parent_link.join("campaign.bin"),
                HARD_MAX_CAMPAIGN_BYTES,
            )
            .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn candidate_campaign_identity_never_reopens_the_ambient_absolute_path() {
        let authenticated_candidate_files = BTreeMap::from([(
            "private/campaign-states/campaign.bin".to_owned(),
            b"candidate bytes".to_vec(),
        )]);

        let ambient = tempfile::tempdir().unwrap();
        let ambient_parent = ambient.path().join("release/private/campaign-states");
        let configured = ambient_parent.join("campaign.bin");
        let expected = (
            Sha256::digest(b"candidate bytes").into(),
            b"candidate bytes".len() as u64,
        );

        // A not-yet-installed candidate has no ambient release pathname.
        assert!(!configured.exists());
        assert_eq!(
            hash_authenticated_candidate_campaign_state(
                &authenticated_candidate_files,
                &configured,
                &ambient_parent,
                HARD_MAX_CAMPAIGN_BYTES,
            )
            .unwrap(),
            expected
        );

        // Even a malicious ambient file, including a replacement between
        // probes, is irrelevant: both fields are derived from the retained FD.
        std::fs::create_dir_all(&ambient_parent).unwrap();
        std::fs::write(&configured, b"malicious short bytes").unwrap();
        assert_eq!(
            hash_authenticated_candidate_campaign_state(
                &authenticated_candidate_files,
                &configured,
                &ambient_parent,
                HARD_MAX_CAMPAIGN_BYTES,
            )
            .unwrap(),
            expected
        );
        std::fs::write(&configured, vec![0xa5; 4096]).unwrap();
        assert_eq!(
            hash_authenticated_candidate_campaign_state(
                &authenticated_candidate_files,
                &configured,
                &ambient_parent,
                HARD_MAX_CAMPAIGN_BYTES,
            )
            .unwrap(),
            expected
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn moderation_token_rejects_symlinked_file_and_parent() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let directory = tempfile::tempdir().unwrap();
        let real_parent = directory.path().join("real");
        std::fs::create_dir(&real_parent).unwrap();
        std::fs::set_permissions(&real_parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        let target = real_parent.join("token");
        std::fs::write(&target, b"0123456789abcdef0123456789abcdef").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o400)).unwrap();
        assert_eq!(
            load_private_bearer_token(&target).unwrap(),
            b"0123456789abcdef0123456789abcdef"
        );

        let file_link = real_parent.join("token-link");
        symlink(&target, &file_link).unwrap();
        assert!(load_private_bearer_token(&file_link).is_err());

        let parent_link = directory.path().join("parent-link");
        symlink(&real_parent, &parent_link).unwrap();
        assert!(load_private_bearer_token(&parent_link.join("token")).is_err());
    }

    #[test]
    fn backup_config_load_uses_ephemeral_moderation_credential_only_for_reading() {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let configured_token = directory.path().join("api-owned-token");
        let credential = directory.path().join("credential-token");
        std::fs::write(&credential, b"0123456789abcdef0123456789abcdef").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o400)).unwrap();
        }

        let original = ServerConfig {
            moderation_bearer_token_path: Some(configured_token.clone()),
            ..Default::default()
        };
        let config_path = directory.path().join("server.toml");
        std::fs::write(&config_path, toml::to_string(&original).unwrap()).unwrap();
        assert!(ServerConfig::load(&config_path).is_err());

        let sources = BTreeMap::from([(configured_token.clone(), credential)]);
        let loaded = ServerConfig::load_with_backup_credentials(&config_path, &sources).unwrap();
        assert_eq!(
            loaded.moderation_bearer_token_path.as_ref(),
            Some(&configured_token)
        );
        assert_eq!(
            loaded.moderation_bearer_token.as_deref().map(Vec::as_slice),
            Some(b"0123456789abcdef0123456789abcdef".as_slice())
        );
    }

    #[test]
    fn worker_config_load_never_opens_api_moderation_token() {
        let directory = tempfile::tempdir().unwrap();
        let missing_token = directory.path().join("api-only-token-does-not-exist");
        let config = ServerConfig {
            moderation_bearer_token_path: Some(missing_token.clone()),
            cursor_secret_path: directory.path().join("cursor-key-must-not-be-opened"),
            competition_run_grant_secret_path: directory
                .path()
                .join("grant-key-must-not-be-opened"),
            run_preflight_grant_secret_path: directory
                .path()
                .join("preflight-key-must-not-be-opened"),
            backup_authority_hmac_secret_path: directory
                .path()
                .join("backup-authority-key-must-not-be-opened"),
            ..Default::default()
        };
        let config_path = directory.path().join("server.toml");
        std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();

        // The API loader proves it tried to open the configured credential.
        assert!(ServerConfig::load(&config_path).is_err());

        // The worker validates the same non-secret document and manifest
        // surface without touching the API-only path.
        let worker = ServerConfig::load_for_worker(&config_path).unwrap();
        assert_eq!(
            worker.moderation_bearer_token_path.as_deref(),
            Some(missing_token.as_path())
        );
        assert!(worker.moderation_bearer_token.is_none());
    }

    #[test]
    fn shipped_user_units_cover_exact_runtime_paths_and_direct_launcher() {
        let api = include_str!("../deploy/robin-highscores-api.service");
        let worker = include_str!("../deploy/robin-highscores-worker.service");
        let backup = include_str!("../deploy/robin-highscores-backup.service");
        let timer = include_str!("../deploy/robin-highscores-backup.timer");
        let server_config = include_str!("../highscores-server.example.toml");
        let worker_config = include_str!("../highscores-worker.example.toml");
        let nginx = include_str!("../deploy/nginx-robinhood-api.locations.conf");
        let api_environment = include_str!("../deploy/api.env.example");
        let worker_environment = include_str!("../deploy/worker.env.example");
        let root_once = include_str!("../deploy/root-once.sh");
        let install = include_str!("../README.md");

        assert_eq!(api_environment, "RUST_LOG=info\n");
        assert_eq!(worker_environment, "RUST_LOG=info\n");

        for path in [
            "/home/robinhood/.local/share/robin-highscores/database/highscores.sqlite3",
            "/home/robinhood/.local/share/robin-highscores/replays",
            "/home/robinhood/.local/share/robin-highscores/campaign-states",
            "/home/robinhood/.local/share/robin-highscores/api-secrets/cursor-hmac.key",
            "/home/robinhood/.local/share/robin-highscores/api-secrets/competition-run-grant.key",
            "/home/robinhood/.local/share/robin-highscores/api-secrets/run-preflight-grant.key",
            "/home/robinhood/.local/share/robin-highscores/api-secrets/backup-authority-hmac.key",
            "/home/robinhood/.local/share/robin-highscores/api-secrets/moderation-bearer.token",
            "/home/robinhood/.local/share/robin-highscores/status/backup-status.json",
        ] {
            assert!(
                server_config.contains(path),
                "example config omits runtime path {path}"
            );
        }
        let state_root = "/home/robinhood/.local/share/robin-highscores";
        for shared_path in ["database", "replays", "campaign-states"] {
            let shared_path = format!("{state_root}/{shared_path}");
            assert!(
                api.contains(&format!("ReadWritePaths={shared_path}")),
                "API unit cannot write required shared state {shared_path}"
            );
            assert!(
                worker.contains(&format!("ReadWritePaths={shared_path}")),
                "worker unit cannot write required shared state {shared_path}"
            );
        }
        assert!(
            backup.contains("ReadWritePaths=/home/robinhood/.local/share/robin-highscores/backups")
        );
        assert!(
            backup.contains("ReadWritePaths=/home/robinhood/.local/share/robin-highscores/status")
        );
        assert!(backup.contains(
            "--status-path /home/robinhood/.local/share/robin-highscores/status/backup-status.json"
        ));
        assert!(backup.contains("backup-and-publish-status"));
        assert!(backup.contains("--retain-complete 2"));
        assert!(server_config.contains("maximum_backup_age_hours = 32"));
        assert!(backup.contains("TimeoutStartSec=6h"));
        assert!(timer.contains("RandomizedDelaySec=45m"));
        assert!(backup.contains(
            "--release-manifest-path /home/robinhood/.local/opt/robin-highscores/releases/@SOURCE_COMMIT@/vps-release-manifest-v2.json"
        ));
        assert!(!backup.contains("--configuration-root"));
        assert!(
            !backup.contains(
                "--restore-source-map /home/robinhood/.local/opt/robin-highscores/releases"
            )
        );
        assert_eq!(backup.matches("--restore-source-map ").count(), 9);
        assert!(!backup.contains(
            "--restore-source-map /home/robinhood/.config/systemd/user=/home/robinhood/.config/systemd/user"
        ));
        assert!(!backup.contains("ReadOnlyPaths=/home/robinhood/.config/systemd/user\n"));
        for unit in [
            "robin-highscores.target",
            "robin-highscores-api.service",
            "robin-highscores-worker.service",
            "robin-highscores-backup.service",
            "robin-highscores-backup.timer",
        ] {
            assert!(backup.contains(&format!(
                "ReadOnlyPaths=/home/robinhood/.config/systemd/user/{unit}"
            )));
        }
        assert!(backup.contains("RestrictAddressFamilies=AF_UNIX"));
        for (name, service) in [("api", api), ("worker", worker), ("backup", backup)] {
            assert_eq!(
                service.matches("\nPrivateUsers=yes\n").count(),
                1,
                "{name} must use a private user namespace so the unprivileged user manager can apply its capability and device hardening"
            );
            assert!(service.contains("\nCapabilityBoundingSet=\n"));
            assert!(service.contains("\nAmbientCapabilities=\n"));
            assert!(service.contains("\nNoNewPrivileges=yes\n"));
            assert!(
                !service.contains("\nRestrictSUIDSGID="),
                "{name} must not combine RestrictSUIDSGID with PrivateUsers because the deployed unprivileged user manager returns ENOSYS for openat2 under that combination"
            );
        }
        assert!(worker.contains("\nRestrictAddressFamilies=AF_UNIX AF_NETLINK\n"));
        assert!(!worker.contains("\nRestrictAddressFamilies=AF_UNIX\n"));
        assert!(timer.contains("Persistent=true"));
        assert!(api.contains("\nType=notify\n"));
        assert!(api.contains("\nNotifyAccess=main\n"));
        assert!(api.contains("\nTimeoutStartSec=15min\n"));
        assert!(!api.contains("\nType=exec\n"));
        assert!(worker.contains("ReadOnlyPaths=/usr/bin/bwrap /usr/bin/prlimit"));
        assert!(worker.contains("\nType=notify\n"));
        assert!(worker.contains("\nNotifyAccess=main\n"));
        assert!(worker.contains("\nTimeoutStartSec=15min\n"));
        assert!(!worker.contains("\nType=exec\n"));
        assert!(worker.contains(&format!("ReadOnlyPaths={state_root}/raw-content")));
        assert!(worker.contains(&format!("InaccessiblePaths={state_root}/api-secrets")));
        assert!(worker.contains(&format!("InaccessiblePaths={state_root}/backups")));
        assert!(worker.contains(&format!("InaccessiblePaths={state_root}/status")));
        assert!(api.contains(&format!("ReadOnlyPaths={state_root}/status")));
        assert!(api.contains(&format!("ReadOnlyPaths={state_root}/runtime-fence")));
        assert!(worker.contains(&format!("ReadOnlyPaths={state_root}/runtime-fence")));
        assert!(backup.contains(&format!("ReadOnlyPaths={state_root}/runtime-fence")));
        assert!(api.contains(&format!("InaccessiblePaths={state_root}/backups")));
        assert!(!api.contains(&format!("ReadOnlyPaths={state_root}/backups")));
        for unit in [api, worker, backup] {
            for obsolete in [
                "\nUser=",
                "\nGroup=",
                "SupplementaryGroups=",
                "verifier-broker",
                "systemd-run",
                "polkit",
                "=/opt/robin-highscores",
                "=/var/lib/robin-highscores",
                "=/srv/robin-highscores",
            ] {
                assert!(!unit.contains(obsolete), "user unit contains {obsolete}");
            }
        }
        for contract in [
            "bwrap_program = \"/usr/bin/bwrap\"",
            "prlimit_program = \"/usr/bin/prlimit\"",
            "[verifier_launcher]",
            "/home/robinhood/.local/opt/robin-highscores/releases/",
            "/home/robinhood/.local/share/robin-highscores/raw-content/{demo,full}",
        ] {
            assert!(
                worker_config.contains(contract),
                "worker config omits {contract}"
            );
        }
        for obsolete in [
            "broker_socket",
            "broker_response_timeout",
            "systemd-run",
            "= \"/opt/robin-highscores",
            "= \"/var/lib/robin-highscores",
            "= \"/srv/robin-highscores",
        ] {
            assert!(
                !worker_config.contains(obsolete),
                "worker config contains {obsolete}"
            );
        }
        assert!(root_once.contains("[ \"$(id -u)\" -eq 0 ]"));
        assert!(root_once.contains("loginctl enable-linger robinhood"));
        assert!(install.contains("As `robinhood`, run exactly one release"));
        let worker_source = include_str!("bin/worker.rs");
        assert!(worker_source.contains("ServerConfig::load_for_worker"));
        assert!(!worker_source.contains(".load_cursor_key"));
        assert!(!worker_source.contains(".load_competition_run_grant_key"));
        assert!(!worker_source.contains(".load_run_preflight_grant_key"));

        let compiled_max = HARD_MAX_REPLAY_BYTES
            .checked_add(HARD_MAX_CAMPAIGN_BYTES)
            .and_then(|value| value.checked_add(HARD_MAX_METADATA_BYTES as u64))
            .and_then(|value| value.checked_add(1024 * 1024))
            .unwrap();
        assert!(compiled_max <= 130_u64 * 1024 * 1024);
        assert!(nginx.contains("client_max_body_size 130m;"));
        assert!(nginx.contains("location = /api"));
        assert!(nginx.contains("location ^~ /api/"));
        assert!(nginx.contains("proxy_set_header X-Forwarded-For $http_cf_connecting_ip;"));
        assert!(nginx.contains("location /"));
        assert!(nginx.contains("return 404;"));
    }

    #[test]
    fn manifestctl_split_ruleset_layout_loads_and_cross_binds() {
        let directory = empty_manifest_registry_directory();
        let published = crate::web::tests::published_ruleset_fixture();
        let digest = published.ruleset_manifest_sha256;
        std::fs::write(
            directory
                .path()
                .join("ruleset-manifests")
                .join(format!("{digest}.json")),
            robin_run_protocol::canonical_json_bytes(&published.manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(
            directory
                .path()
                .join("published-rulesets")
                .join(format!("{digest}.json")),
            robin_run_protocol::canonical_json_bytes(&published).unwrap(),
        )
        .unwrap();

        let registry = ManifestRegistry::load(directory.path()).unwrap();
        assert_eq!(registry.rulesets.get(&digest), Some(&published));

        std::fs::remove_file(
            directory
                .path()
                .join("ruleset-manifests")
                .join(format!("{digest}.json")),
        )
        .unwrap();
        assert!(ManifestRegistry::load(directory.path()).is_err());
    }

    #[test]
    fn build_registry_preserves_exact_v2_identity_and_separate_semantics() {
        let directory = empty_manifest_registry_directory();
        let build = crate::web::tests::viewer_build_v2();
        let document = VersionedBuildManifest::V2(build.clone());
        let public_digest = document.canonical_digest().unwrap();
        let semantic = document.backend_visible_v1().unwrap();
        let semantic_digest = semantic.canonical_digest().unwrap();
        assert_ne!(public_digest, semantic_digest);
        write_build_document(directory.path(), public_digest, &document);

        let registry = ManifestRegistry::load(directory.path()).unwrap();
        let loaded = registry.builds.get(&public_digest).unwrap();
        assert_eq!(loaded.public_document(), &document);
        assert_eq!(loaded.public_digest(), public_digest);
        assert_eq!(loaded.semantics(), &semantic);
        assert_eq!(loaded.semantic_digest(), semantic_digest);
        assert!(!registry.builds.contains_key(&semantic_digest));
        let decoded: LoadedBuildManifest =
            serde_json::from_slice(&loaded.public_document().canonical_bytes().unwrap()).unwrap();
        assert_eq!(&decoded, loaded);
    }

    #[test]
    fn build_registry_rejects_wrong_filename_private_fields_and_duplicate_semantics() {
        let build = crate::web::tests::viewer_build_v2();
        let document = VersionedBuildManifest::V2(build.clone());
        let public_digest = document.canonical_digest().unwrap();

        let wrong_name = empty_manifest_registry_directory();
        write_build_document(wrong_name.path(), Digest32::from_bytes([99; 32]), &document);
        assert!(ManifestRegistry::load(wrong_name.path()).is_err());

        for private_field in [
            "projection_exporter",
            "projection_authority",
            "projection_receipt",
            "source_tree_manifest",
            "unexpected_public_field",
        ] {
            let hostile = empty_manifest_registry_directory();
            let mut value = serde_json::to_value(&document).unwrap();
            value.as_object_mut().unwrap().insert(
                private_field.to_owned(),
                serde_json::json!({"sha256": Digest32::from_bytes([98; 32])}),
            );
            assert!(
                serde_json::from_value::<VersionedBuildManifest>(value.clone()).is_err(),
                "accepted private build field {private_field}"
            );
            std::fs::write(
                hostile
                    .path()
                    .join("builds")
                    .join(format!("{public_digest}.json")),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
            assert!(
                ManifestRegistry::load(hostile.path()).is_err(),
                "loaded private build field {private_field}"
            );
        }

        let nested_hostile = empty_manifest_registry_directory();
        let mut nested = serde_json::to_value(&document).unwrap();
        nested["viewer"]["engine"].as_object_mut().unwrap().insert(
            "projection_exporter".to_owned(),
            serde_json::json!({"artifact_sha256": Digest32::from_bytes([97; 32])}),
        );
        assert!(serde_json::from_value::<VersionedBuildManifest>(nested.clone()).is_err());
        std::fs::write(
            nested_hostile
                .path()
                .join("builds")
                .join(format!("{public_digest}.json")),
            serde_json::to_vec(&nested).unwrap(),
        )
        .unwrap();
        assert!(ManifestRegistry::load(nested_hostile.path()).is_err());

        let duplicate = empty_manifest_registry_directory();
        write_build_document(duplicate.path(), public_digest, &document);
        let historical = VersionedBuildManifest::V1(build.backend_visible_v1().unwrap());
        let historical_digest = historical.canonical_digest().unwrap();
        write_build_document(duplicate.path(), historical_digest, &historical);
        assert!(ManifestRegistry::load(duplicate.path()).is_err());
    }

    #[test]
    fn current_ranked_build_requires_v2_and_stale_build_requires_quarantine() {
        let build_v2 = crate::web::tests::viewer_build_v2();
        let mut build = LoadedBuildManifest::new(VersionedBuildManifest::V2(build_v2)).unwrap();
        assert_eq!(
            build.semantics().save_schema_version,
            robin_run_protocol::CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1
        );
        let published = crate::web::tests::published_ruleset_fixture();
        validate_current_ranked_build(&build, &published).unwrap();

        assert_eq!(
            build.semantics().network_protocol_version,
            robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1
        );
        let mut stale_network = build.public_document().clone();
        match &mut stale_network {
            VersionedBuildManifest::V2(stale) => stale.network_protocol_version -= 1,
            VersionedBuildManifest::V1(_) => unreachable!(),
        }
        let stale_network = LoadedBuildManifest::new(stale_network).unwrap();
        assert!(validate_current_ranked_build(&stale_network, &published).is_err());

        let mut stale_ruleset_network = published.clone();
        stale_ruleset_network.manifest.network_protocol_versions =
            vec![robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1 - 1];
        assert!(validate_current_ranked_build(&build, &stale_ruleset_network).is_err());

        let mut stale = build.public_document().clone();
        match &mut stale {
            VersionedBuildManifest::V2(stale) => stale.save_schema_version -= 1,
            VersionedBuildManifest::V1(_) => unreachable!(),
        }
        build = LoadedBuildManifest::new(stale).unwrap();
        assert!(validate_current_ranked_build(&build, &published).is_err());
        let mut quarantined = published;
        quarantined.operational_status = RulesetOperationalStatusV1::Quarantined {
            audit_id: robin_run_protocol::OpaqueId::new("stale-build-audit").unwrap(),
            reason_code: "stale-save-schema".to_owned(),
            since_unix_ms: 1,
        };
        validate_current_ranked_build(&build, &quarantined).unwrap();

        let historical =
            LoadedBuildManifest::new(VersionedBuildManifest::V1(crate::web::tests::viewer_build()))
                .unwrap();
        assert!(validate_current_ranked_build(&historical, &quarantined).is_ok());
        quarantined.operational_status = RulesetOperationalStatusV1::Active;
        assert!(validate_current_ranked_build(&historical, &quarantined).is_err());
    }
}
