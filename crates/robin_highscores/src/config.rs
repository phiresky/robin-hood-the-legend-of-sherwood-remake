use robin_run_protocol::{
    BoardSimulationPolicyV1, BoardV2, LeaderboardMetadataV2, OfficialContentEditionV1, OpaqueId,
    SCHEMA_VERSION_V2, SIGNED_REQUEST_DEFAULT_MAX_AGE_MS,
    SIGNED_REQUEST_DEFAULT_MAX_FUTURE_SKEW_MS, SignedRequestWindowV1, TickDurationV1,
    Validate as _, ViewerContentRequirementV2,
};
use serde::{Deserialize, Serialize};
use std::io::Read as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

/// The API transport cap is the canonical compact codec's input cap. The API
/// uses its allocation-free header preflight, but must never invoke zstd, bitcode, or typed validation on hostile upload bytes.
pub const HARD_MAX_REPLAY_BYTES: u64 =
    robin_replay_format::DEFAULT_REPLAY_ADMISSION_LIMITS.max_input_bytes as u64;
pub const HARD_MAX_METADATA_BYTES: usize = 256 * 1024;
pub const HARD_MAX_PAGE_SIZE: u32 = 100;
const HARD_MAX_OPERATOR_DOCUMENT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub database_path: PathBuf,
    pub replay_directory: PathBuf,
    pub cursor_secret_path: PathBuf,
    pub moderation_bearer_token_path: Option<PathBuf>,
    pub moderation_operator_id: String,
    #[serde(skip)]
    pub moderation_bearer_token: Option<std::sync::Arc<Vec<u8>>>,
    pub allowed_origins: Vec<String>,
    /// Only peers in these networks may supply X-Forwarded-For. A matching
    /// peer must supply exactly one canonical IP address or the request fails
    /// closed; untrusted peers' forwarding headers are ignored.
    pub trusted_proxy_cidrs: Vec<String>,
    /// Per effective client address, per operation: username updates,
    /// deletion requests and private submission status reads.
    pub signed_requests_per_minute_per_ip: u32,
    /// Submission attempts per effective client address, checked before the
    /// body is parsed.
    pub submissions_per_hour_per_ip: u32,
    /// Submission attempts per uploader key, checked after signature
    /// verification.
    pub submissions_per_hour_per_key: u32,
    /// Uploads holding a live reservation lease per uploader key.
    pub max_concurrent_uploads_per_key: u32,
    pub abuse_reports_per_hour_per_ip: u32,
    pub abuse_reports_per_hour_per_key: u32,
    pub abuse_reports_per_hour_per_target: u32,
    pub max_replay_bytes: u64,
    pub max_metadata_bytes: usize,
    pub max_pending_submissions: u32,
    pub max_concurrent_requests: usize,
    pub max_concurrent_uploads: usize,
    pub upload_timeout_seconds: u64,
    /// Durable window in which the uploader may resume a reserved or crash-
    /// abandoned upload of the same replay.
    pub upload_reservation_ttl_seconds: u64,
    pub max_page_size: u32,
    pub database_busy_timeout_ms: u64,
    pub tombstone_retention_days: Option<u64>,
    pub rejected_replay_retention_hours: u64,
    pub orphan_replay_retention_hours: u64,
    /// Free bytes which must remain after the full bounded admission plan.
    /// Production validation keeps this at or above one GiB.
    pub minimum_storage_free_bytes: u64,
    /// Acceptance window for `signed_at_unix_ms` of every player-signed
    /// request. Kept after all scalar keys so it serializes as a TOML table.
    pub signed_requests: SignedRequestConfig,
    /// Ranked boards. Empty is fail-closed: metadata lists no boards and every
    /// upload is rejected.
    pub boards: Vec<BoardV2>,
}

/// `[signed_requests]`: how far a player's signing clock may lag behind or run
/// ahead of the server when a request arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SignedRequestConfig {
    pub max_age_seconds: u64,
    pub max_future_skew_seconds: u64,
}

impl Default for SignedRequestConfig {
    fn default() -> Self {
        Self {
            max_age_seconds: SIGNED_REQUEST_DEFAULT_MAX_AGE_MS / 1_000,
            max_future_skew_seconds: SIGNED_REQUEST_DEFAULT_MAX_FUTURE_SKEW_MS / 1_000,
        }
    }
}

impl SignedRequestConfig {
    /// Validated bounds keep the millisecond products far from overflow.
    pub fn window(self) -> SignedRequestWindowV1 {
        SignedRequestWindowV1 {
            max_age_ms: self.max_age_seconds.saturating_mul(1_000),
            max_future_skew_ms: self.max_future_skew_seconds.saturating_mul(1_000),
        }
    }

    fn validate(self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (30..=60 * 60).contains(&self.max_age_seconds),
            "signed_requests.max_age_seconds must be between 30 seconds and one hour"
        );
        anyhow::ensure!(
            self.max_future_skew_seconds <= 10 * 60,
            "signed_requests.max_future_skew_seconds must not exceed ten minutes"
        );
        Ok(())
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787),
            database_path: PathBuf::from("data/highscores.sqlite3"),
            replay_directory: PathBuf::from("data/replays"),
            cursor_secret_path: PathBuf::from("data/cursor-hmac.key"),
            moderation_bearer_token_path: None,
            moderation_operator_id: "operator".to_owned(),
            moderation_bearer_token: None,
            allowed_origins: Vec::new(),
            trusted_proxy_cidrs: Vec::new(),
            signed_requests_per_minute_per_ip: 120,
            submissions_per_hour_per_ip: 120,
            submissions_per_hour_per_key: 60,
            max_concurrent_uploads_per_key: 2,
            abuse_reports_per_hour_per_ip: 10,
            abuse_reports_per_hour_per_key: 25,
            abuse_reports_per_hour_per_target: 10,
            max_replay_bytes: 16 * 1024 * 1024,
            max_metadata_bytes: 64 * 1024,
            max_pending_submissions: 10_000,
            max_concurrent_requests: 256,
            max_concurrent_uploads: 32,
            upload_timeout_seconds: 120,
            upload_reservation_ttl_seconds: 30 * 60,
            max_page_size: 100,
            database_busy_timeout_ms: 5_000,
            tombstone_retention_days: Some(30),
            rejected_replay_retention_hours: 24,
            orphan_replay_retention_hours: 24,
            minimum_storage_free_bytes: 1024 * 1024 * 1024,
            signed_requests: SignedRequestConfig::default(),
            boards: Vec::new(),
        }
    }
}

/// Exact duration of one simulation tick, taken from the compiled engine.
pub fn simulation_tick_duration() -> TickDurationV1 {
    TickDurationV1 {
        numerator_micros: u64::from(robin_run_protocol::FRAME_TIME_MS) * 1_000,
        denominator: 1,
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
        Self::load_with_secret_scope(path, ConfigSecretScope::Api)
    }

    /// Load the public/runtime subset needed by the verification worker. The
    /// moderation-token path remains parsed but is never opened.
    pub fn load_for_worker(path: &Path) -> anyhow::Result<Self> {
        Self::load_with_secret_scope(path, ConfigSecretScope::Worker)
    }

    fn load_with_secret_scope(path: &Path, scope: ConfigSecretScope) -> anyhow::Result<Self> {
        let bytes = read_regular_file_no_symlinks(path, HARD_MAX_OPERATOR_DOCUMENT_BYTES)?;
        let mut config: Self = toml::from_str(std::str::from_utf8(&bytes)?)?;
        config.moderation_bearer_token = match scope {
            ConfigSecretScope::Api => config
                .moderation_bearer_token_path
                .as_deref()
                .map(load_private_bearer_token)
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
        self.validate_limits()?;
        self.validate_paths_and_secrets(scope)?;
        self.validate_cors_origins()?;
        self.validate_network_limits()?;
        self.leaderboard_metadata()?;
        Ok(())
    }

    /// The published board catalog, in canonical board-ID order. Loading a
    /// configuration validates this exact document, so serving it cannot fail
    /// for a loaded configuration.
    pub fn leaderboard_metadata(&self) -> anyhow::Result<LeaderboardMetadataV2> {
        let mut boards = self.boards.clone();
        boards.sort_by(|left, right| left.board_id.cmp(&right.board_id));
        for pair in boards.windows(2) {
            anyhow::ensure!(
                pair[0].board_id != pair[1].board_id,
                "duplicate board ID: {}",
                pair[0].board_id
            );
        }
        for board in &boards {
            validate_board(board)?;
        }
        let metadata = LeaderboardMetadataV2 {
            schema_version: SCHEMA_VERSION_V2,
            tick_duration: simulation_tick_duration(),
            boards,
        };
        metadata.validate()?;
        Ok(metadata)
    }

    pub fn board(&self, board_id: &OpaqueId) -> Option<&BoardV2> {
        self.boards.iter().find(|board| &board.board_id == board_id)
    }

    /// Board IDs whose runs are publicly visible. A run whose board was
    /// removed from configuration is hidden rather than reinterpreted.
    pub fn board_ids(&self) -> Vec<String> {
        self.boards
            .iter()
            .map(|board| board.board_id.as_str().to_owned())
            .collect()
    }

    fn validate_limits(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.max_replay_bytes > 0,
            "max_replay_bytes must be positive"
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
            self.max_replay_bytes <= HARD_MAX_REPLAY_BYTES,
            "max_replay_bytes exceeds the compiled safety limit of {HARD_MAX_REPLAY_BYTES}"
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
            (1..=self.max_concurrent_uploads).contains(
                &usize::try_from(self.max_concurrent_uploads_per_key)
                    .map_err(|_| anyhow::anyhow!("max_concurrent_uploads_per_key overflows"))?
            ),
            "max_concurrent_uploads_per_key must be in 1..=max_concurrent_uploads"
        );
        self.signed_requests.validate()?;
        Ok(())
    }

    fn validate_paths_and_secrets(&self, scope: ConfigSecretScope) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.database_path.file_name().is_some(),
            "database_path must name a file"
        );
        anyhow::ensure!(
            self.replay_directory.file_name().is_some(),
            "replay_directory must not be a filesystem root"
        );
        anyhow::ensure!(
            self.cursor_secret_path.file_name().is_some(),
            "cursor_secret_path must name a file"
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
        Ok(())
    }

    fn validate_cors_origins(&self) -> anyhow::Result<()> {
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
        Ok(())
    }

    fn validate_network_limits(&self) -> anyhow::Result<()> {
        for (name, value) in [
            (
                "signed_requests_per_minute_per_ip",
                self.signed_requests_per_minute_per_ip,
            ),
            (
                "submissions_per_hour_per_ip",
                self.submissions_per_hour_per_ip,
            ),
            (
                "submissions_per_hour_per_key",
                self.submissions_per_hour_per_key,
            ),
        ] {
            anyhow::ensure!((1..=10_000).contains(&value), "{name} must be in 1..=10000");
        }
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
}

/// Protocol validation plus the cross-field rules a board document cannot
/// express on its own.
fn validate_board(board: &BoardV2) -> anyhow::Result<()> {
    board
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid board {}: {error}", board.board_id))?;
    anyhow::ensure!(
        matches!(
            (board.edition, board.viewer_content_requirement),
            (
                OfficialContentEditionV1::Demo,
                ViewerContentRequirementV2::BundledDemo
            ) | (
                OfficialContentEditionV1::Full,
                ViewerContentRequirementV2::UserLocalRetail
            )
        ),
        "board {} viewer content requirement does not match its {:?} edition",
        board.board_id,
        board.edition
    );
    if let BoardSimulationPolicyV1::Fixed { policy } = board.simulation_policy {
        anyhow::ensure!(
            board.preset_id == policy.preset.preset_id()
                && board.difficulty_id == policy.difficulty.difficulty_id(),
            "board {} preset/difficulty labels do not match its fixed simulation policy",
            board.board_id
        );
    }
    Ok(())
}

use crate::secure_fs::read_bounded_no_symlinks as read_regular_file_no_symlinks;

fn load_private_bearer_token(path: &Path) -> anyhow::Result<Vec<u8>> {
    use rustix::fs::OFlags;
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
    let parent_fd = crate::secure_fs::open_dir_no_symlinks(parent).map_err(|error| {
        anyhow::anyhow!(
            "moderation bearer token parent must pre-exist without symlinks ({}): {error}",
            parent.display()
        )
    })?;
    let parent_file = std::fs::File::from(parent_fd);
    let token_fd = crate::secure_fs::open_beneath_no_symlinks(
        &parent_file,
        filename,
        OFlags::RDONLY | OFlags::CLOEXEC,
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

fn private_key(path: &Path, create_if_missing: bool, label: &str) -> anyhow::Result<[u8; 32]> {
    use rustix::fs::{Mode, OFlags};
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{label} must have a parent directory"))?;
    let filename = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("{label} must name a file"))?;
    let parent_fd = crate::secure_fs::open_dir_no_symlinks(parent).map_err(|error| {
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
        let fd = crate::secure_fs::open_beneath_no_symlinks(
            &parent_file,
            filename,
            OFlags::RDONLY | OFlags::CLOEXEC,
        )?;
        Ok(std::fs::File::from(fd))
    };

    let mut file = if create_if_missing {
        match crate::secure_fs::create_beneath_no_symlinks(
            &parent_file,
            filename,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o400),
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

#[cfg(test)]
mod tests {
    use super::*;
    use robin_run_protocol::{
        BoardMetricV1, BoardMissionV2, RankedSimulationDifficultyV1, RankedSimulationPolicyV1,
    };

    fn board(id: &str) -> BoardV2 {
        BoardV2 {
            board_id: OpaqueId::new(id).unwrap(),
            display_name: "Demo / Standard / Normal".into(),
            edition: OfficialContentEditionV1::Demo,
            preset_id: "standard".into(),
            preset_name: "Standard".into(),
            difficulty_id: "normal".into(),
            difficulty_name: "Normal".into(),
            simulation_policy: BoardSimulationPolicyV1::Fixed {
                policy: RankedSimulationPolicyV1::standard(RankedSimulationDifficultyV1::Medium),
            },
            allow_state_load: false,
            metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
            viewer_content_requirement: ViewerContentRequirementV2::BundledDemo,
            missions: vec![BoardMissionV2 {
                mission_id: "Dem_Lei_MP".into(),
                display_name: "Leicester".into(),
            }],
        }
    }

    #[test]
    fn boards_parse_from_toml_and_are_validated_at_load() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("server.toml");
        std::fs::write(
            &path,
            r#"
database_path = "/tmp/highscores.sqlite3"

[[boards]]
board_id = "full-any"
display_name = "Full / Any settings"
edition = "full"
preset_id = "any"
preset_name = "Any"
difficulty_id = "any"
difficulty_name = "Any"
simulation_policy = { kind = "any_config" }
allow_state_load = false
metrics = ["original_score", "fastest_success"]
viewer_content_requirement = "user_local_retail"
missions = [{ mission_id = "H01_Lin_VL", display_name = "Lincoln" }]

[[boards]]
board_id = "demo-standard-normal"
display_name = "Demo / Standard / Normal"
edition = "demo"
preset_id = "standard"
preset_name = "Standard"
difficulty_id = "normal"
difficulty_name = "Normal"
simulation_policy = { kind = "fixed", policy = { version = 1, preset = "standard", difficulty = "medium" } }
allow_state_load = false
metrics = ["original_score", "fastest_success"]
viewer_content_requirement = "bundled_demo"
missions = [{ mission_id = "Dem_Lei_MP", display_name = "Leicester" }]
"#,
        )
        .unwrap();
        let config = ServerConfig::load_for_worker(&path).unwrap();
        assert_eq!(config.boards.len(), 2);
        let metadata = config.leaderboard_metadata().unwrap();
        assert_eq!(
            metadata
                .boards
                .iter()
                .map(|board| board.board_id.as_str())
                .collect::<Vec<_>>(),
            ["demo-standard-normal", "full-any"]
        );
        assert_eq!(
            config.boards[0].simulation_policy,
            BoardSimulationPolicyV1::AnyConfig
        );
        assert_eq!(metadata.tick_duration.numerator_micros, 40_000);

        let unknown = std::fs::read_to_string(&path).unwrap().replace(
            "allow_state_load = false\nmetrics",
            "surprise = 1\nallow_state_load = false\nmetrics",
        );
        std::fs::write(&path, unknown).unwrap();
        assert!(ServerConfig::load_for_worker(&path).is_err());
    }

    #[test]
    fn duplicate_invalid_and_mislabelled_boards_are_rejected() {
        let config = ServerConfig {
            boards: vec![board("demo-standard-normal")],
            ..Default::default()
        };
        config.validate().unwrap();

        let duplicate = ServerConfig {
            boards: vec![board("demo-standard-normal"), board("demo-standard-normal")],
            ..Default::default()
        };
        assert!(duplicate.validate().is_err());

        let mut no_missions = board("empty");
        no_missions.missions.clear();
        assert!(
            ServerConfig {
                boards: vec![no_missions],
                ..Default::default()
            }
            .validate()
            .is_err()
        );

        let mut wrong_viewer = board("wrong-viewer");
        wrong_viewer.viewer_content_requirement = ViewerContentRequirementV2::UserLocalRetail;
        assert!(
            ServerConfig {
                boards: vec![wrong_viewer],
                ..Default::default()
            }
            .validate()
            .is_err()
        );

        let mut wrong_label = board("wrong-label");
        wrong_label.difficulty_id = "hard".into();
        assert!(
            ServerConfig {
                boards: vec![wrong_label],
                ..Default::default()
            }
            .validate()
            .is_err()
        );

        let mut unsorted_metrics = board("unsorted");
        unsorted_metrics.metrics.reverse();
        assert!(
            ServerConfig {
                boards: vec![unsorted_metrics],
                ..Default::default()
            }
            .validate()
            .is_err()
        );
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

        let mut config = ServerConfig::default();
        config.max_concurrent_uploads_per_key = 0;
        assert!(config.validate().is_err());
        config.max_concurrent_uploads_per_key =
            u32::try_from(config.max_concurrent_uploads + 1).unwrap();
        assert!(config.validate().is_err());

        let config = ServerConfig {
            submissions_per_hour_per_key: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());

        for (max_age_seconds, max_future_skew_seconds, valid) in [
            (300, 60, true),
            (29, 60, false),
            (3_601, 60, false),
            (300, 601, false),
        ] {
            let config = ServerConfig {
                signed_requests: SignedRequestConfig {
                    max_age_seconds,
                    max_future_skew_seconds,
                },
                ..Default::default()
            };
            assert_eq!(
                config.validate().is_ok(),
                valid,
                "{max_age_seconds}/{max_future_skew_seconds}"
            );
        }
        assert_eq!(
            SignedRequestConfig::default().window(),
            SignedRequestWindowV1::default()
        );

        for (origin, valid) in [
            ("http://example.com", false),
            ("http://localhost.evil.example", false),
            ("http://127.0.0.1:3000", true),
            ("http://127.0.0.2:3000", false),
        ] {
            let config = ServerConfig {
                allowed_origins: vec![origin.to_owned()],
                ..Default::default()
            };
            assert_eq!(config.validate().is_ok(), valid, "{origin}");
        }
    }

    #[test]
    fn cursor_secret_is_durable_and_exact_length() {
        let directory = tempfile::tempdir().unwrap();
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let config = ServerConfig {
            cursor_secret_path: directory.path().join("cursor.key"),
            ..Default::default()
        };
        assert!(config.load_cursor_key().is_err());
        let first = config.load_or_create_cursor_key().unwrap();
        assert_eq!(config.load_cursor_key().unwrap(), first);
        assert_eq!(config.load_or_create_cursor_key().unwrap(), first);
        assert_eq!(std::fs::read(&config.cursor_secret_path).unwrap().len(), 32);
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
    }

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
    fn worker_config_load_never_opens_api_moderation_token() {
        let directory = tempfile::tempdir().unwrap();
        let missing_token = directory.path().join("api-only-token-does-not-exist");
        let config = ServerConfig {
            moderation_bearer_token_path: Some(missing_token.clone()),
            cursor_secret_path: directory.path().join("cursor-key-must-not-be-opened"),
            ..Default::default()
        };
        let config_path = directory.path().join("server.toml");
        std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();

        assert!(ServerConfig::load(&config_path).is_err());
        let worker = ServerConfig::load_for_worker(&config_path).unwrap();
        assert_eq!(
            worker.moderation_bearer_token_path.as_deref(),
            Some(missing_token.as_path())
        );
        assert!(worker.moderation_bearer_token.is_none());
    }

    #[test]
    fn example_and_production_server_configs_load() {
        for (name, text) in [
            ("example", include_str!("../highscores-server.example.toml")),
            ("production", include_str!("../ops/production/server.toml")),
        ] {
            let config: ServerConfig =
                toml::from_str(text).unwrap_or_else(|error| panic!("{name}: {error}"));
            config
                .validate_for_secret_scope(ConfigSecretScope::Worker)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
        }
        let production: ServerConfig =
            toml::from_str(include_str!("../ops/production/server.toml")).unwrap();
        assert!(
            production
                .board(&OpaqueId::new("full-any").unwrap())
                .is_none()
        );
        let full_any = production
            .board(&OpaqueId::new("full-standard-normal").unwrap())
            .unwrap();
        assert_eq!(full_any.missions.len(), 38);
        assert!(full_any.mission("Sherwood").is_none());
        assert!(full_any.mission("SherwoodOutro").is_some());
        assert_eq!(production.boards.len(), 13);
    }

    #[test]
    fn shipped_user_units_cover_exact_runtime_paths_and_direct_launcher() {
        let api = include_str!("../ops/systemd/robin-highscores-api.service");
        let worker = include_str!("../ops/systemd/robin-highscores-worker.service");
        let backup = include_str!("../ops/systemd/robin-highscores-backup.service");
        let timer = include_str!("../ops/systemd/robin-highscores-backup.timer");
        let server_config = include_str!("../highscores-server.example.toml");
        let worker_config = include_str!("../highscores-worker.example.toml");
        let nginx = include_str!("../deploy/nginx-robinhood-api.locations.conf");
        let api_environment = include_str!("../deploy/api.env.example");
        let worker_environment = include_str!("../deploy/worker.env.example");

        assert_eq!(api_environment, "RUST_LOG=info\n");
        assert_eq!(worker_environment, "RUST_LOG=info\n");

        for path in [
            "/home/robinhood/.local/share/robin-highscores/database/highscores.sqlite3",
            "/home/robinhood/.local/share/robin-highscores/replays",
            "/home/robinhood/.local/share/robin-highscores/api-secrets/cursor-hmac.key",
            "/home/robinhood/.local/share/robin-highscores/api-secrets/moderation-bearer.token",
        ] {
            assert!(
                server_config.contains(path),
                "example config omits runtime path {path}"
            );
        }
        assert!(api.contains(
            "\nExecStart=%h/.local/opt/robin-highscores/current/bin/robin-highscores-server --config %h/.config/robin-highscores/server.toml\n"
        ));
        assert!(worker.contains(
            "\nExecStart=%h/.local/opt/robin-highscores/current/bin/robin-highscores-worker --config %h/.config/robin-highscores/worker.toml\n"
        ));
        assert!(
            backup.contains("\nExecStart=%h/.local/opt/robin-highscores/current/ops/backup.sh\n")
        );
        let state_root = "%h/.local/share/robin-highscores";
        for shared_path in ["database", "replays"] {
            let shared_path = format!("{state_root}/{shared_path}");
            assert!(
                api.contains(&format!("ReadWritePaths={shared_path}\n")),
                "API unit cannot write required shared state {shared_path}"
            );
            assert!(
                worker.contains(&format!("ReadWritePaths={shared_path}\n")),
                "worker unit cannot write required shared state {shared_path}"
            );
        }
        for unit in [api, worker, backup] {
            assert!(!unit.contains("campaign-states"));
        }
        // One writable mount: `cp -al` cannot hard-link across bind mounts.
        assert!(backup.contains(&format!("\nReadWritePaths={state_root}\n")));
        assert!(timer.contains("Persistent=true"));
        for (name, service) in [("api", api), ("worker", worker)] {
            assert_eq!(
                service.matches("\nPrivateUsers=yes\n").count(),
                1,
                "{name} must use a private user namespace"
            );
            assert!(service.contains("\nCapabilityBoundingSet=\n"));
            assert!(service.contains("\nAmbientCapabilities=\n"));
            assert!(service.contains("\nNoNewPrivileges=yes\n"));
            assert!(service.contains("\nType=notify\n"));
            assert!(service.contains("\nNotifyAccess=main\n"));
            // The deployed user manager returns ENOSYS for openat2 when
            // RestrictSUIDSGID is combined with PrivateUsers.
            assert_eq!(
                service.matches("\nRestrictSUIDSGID=").count(),
                service.matches("\nRestrictSUIDSGID=no\n").count(),
                "{name} must leave RestrictSUIDSGID off"
            );
            assert!(service.contains(&format!("InaccessiblePaths={state_root}/backups")));
        }
        for unit in [api, worker, backup, timer] {
            for obsolete in [
                "\nUser=",
                "\nGroup=",
                "SupplementaryGroups=",
                "@SOURCE_COMMIT@",
                "runtime-fence",
                "/status",
                "/releases/",
                "/home/robinhood",
            ] {
                assert!(!unit.contains(obsolete), "user unit contains {obsolete}");
            }
        }
        assert!(api.contains("\nTimeoutStartSec=15min\n"));
        assert!(worker.contains("\nRestrictAddressFamilies=AF_UNIX AF_NETLINK\n"));
        assert!(
            !worker.contains("\nRestrictNamespaces="),
            "bubblewrap needs unprivileged user namespaces"
        );
        assert!(worker.contains(&format!("InaccessiblePaths={state_root}/api-secrets")));
        for contract in [
            "bwrap_program = \"/usr/bin/bwrap\"",
            "prlimit_program = \"/usr/bin/prlimit\"",
            "[verifier_launcher]",
            "/home/robinhood/.local/opt/robin-highscores/authority/bin/robin-replay-verifier",
            "/home/robinhood/.local/share/robin-highscores/raw-content/demo",
            "/home/robinhood/.local/share/robin-highscores/raw-content/full",
        ] {
            assert!(
                worker_config.contains(contract),
                "worker config omits {contract}"
            );
        }
        for obsolete in [
            "_sha256",
            "catalog",
            "campaign_state_directory",
            "source-tree",
            "systemd-run",
        ] {
            assert!(
                !worker_config.contains(obsolete),
                "worker config contains {obsolete}"
            );
        }
        let worker_source = include_str!("bin/worker.rs");
        assert!(worker_source.contains("ServerConfig::load_for_worker"));
        assert!(!worker_source.contains(".load_cursor_key"));

        let compiled_max = HARD_MAX_REPLAY_BYTES
            .checked_add(HARD_MAX_METADATA_BYTES as u64)
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
}
