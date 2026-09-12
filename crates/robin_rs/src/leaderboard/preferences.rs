//! Host-only preferences for verified leaderboards.
//!
//! Automatic submission is deliberately opt-in and defaults off. The public
//! deployment endpoint is not a user-editable preference: browser production
//! uses same-origin `/api/v1`, native production uses the corresponding HTTPS
//! origin, and debug builds may opt into an exact loopback endpoint through an
//! environment override.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SAME_ORIGIN_API_BASE_PATH: &str = "/api/v1";
pub const NATIVE_PRODUCTION_API_BASE_URL: &str = "https://robinhood.phiresky.xyz/api/v1";
pub const LOCAL_API_BASE_URL: &str = "http://127.0.0.1:8787/api/v1";
pub const API_BASE_URL_ENV: &str = "ROBINHOOD_LEADERBOARD_API_URL";
const PREFERENCES_FILE: &str = "leaderboards.json";
const BROWSER_PREFERENCES_KEY: &str = "robin-hood.leaderboards.v1";
const PREFERENCES_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LeaderboardScope {
    #[default]
    IndividualLevel,
    Campaign,
    FullCampaign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LeaderboardTab {
    #[default]
    Score,
    Time,
    Challenge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeaderboardPreferences {
    pub schema_version: u16,
    /// Show the non-blocking board panel after every completed mission,
    /// including losses and interrupted attempts. This controls presentation
    /// only; replay recording and canonical campaign history remain active.
    #[serde(default = "default_true")]
    pub show_mission_end_boards: bool,
    /// Explicit consent to submit every eligible won mission. A missing or
    /// malformed store can never turn this on.
    #[serde(default)]
    pub always_submit_eligible_runs: bool,
    #[serde(default)]
    pub preferred_scope: LeaderboardScope,
    #[serde(default)]
    pub preferred_tab: LeaderboardTab,
    #[serde(default)]
    pub preferred_preset_id: Option<String>,
    #[serde(default)]
    pub preferred_difficulty_id: Option<String>,
    #[serde(default)]
    pub preferred_competition_id: Option<String>,
    #[serde(default = "default_preferred_player_count")]
    pub preferred_max_concurrent_players: Option<u16>,
}

impl Default for LeaderboardPreferences {
    fn default() -> Self {
        Self {
            schema_version: PREFERENCES_SCHEMA_VERSION,
            show_mission_end_boards: true,
            always_submit_eligible_runs: false,
            preferred_scope: LeaderboardScope::IndividualLevel,
            preferred_tab: LeaderboardTab::Score,
            preferred_preset_id: None,
            preferred_difficulty_id: None,
            preferred_competition_id: None,
            preferred_max_concurrent_players: Some(1),
        }
    }
}

impl LeaderboardPreferences {
    pub fn validate(&self) -> Result<(), LeaderboardPreferencesError> {
        if self.schema_version != PREFERENCES_SCHEMA_VERSION {
            return Err(LeaderboardPreferencesError::UnsupportedSchema {
                found: self.schema_version,
                expected: PREFERENCES_SCHEMA_VERSION,
            });
        }
        validate_optional_facet("preferred_preset_id", self.preferred_preset_id.as_deref())?;
        validate_optional_facet(
            "preferred_difficulty_id",
            self.preferred_difficulty_id.as_deref(),
        )?;
        validate_optional_facet(
            "preferred_competition_id",
            self.preferred_competition_id.as_deref(),
        )?;
        if self
            .preferred_max_concurrent_players
            .is_some_and(|count| count == 0 || count > robin_run_protocol::MAX_REPLAY_SEATS_V1)
        {
            return Err(LeaderboardPreferencesError::InvalidPlayerCount);
        }
        Ok(())
    }

    pub fn automatically_submit(&self, run_is_eligible_and_won: bool) -> bool {
        self.always_submit_eligible_runs && run_is_eligible_and_won
    }

    pub fn effective_api_base_url(
        &self,
    ) -> Result<LeaderboardApiBaseUrl, LeaderboardPreferencesError> {
        if cfg!(debug_assertions) {
            match std::env::var(API_BASE_URL_ENV) {
                Ok(value) => return LeaderboardApiBaseUrl::parse_development(&value),
                Err(std::env::VarError::NotPresent) => {}
                Err(error) => {
                    return Err(LeaderboardPreferencesError::Environment(error.to_string()));
                }
            }
        }
        Ok(LeaderboardApiBaseUrl::production())
    }
}

/// Validated API base. Relative URLs are restricted to the one production
/// same-origin path; absolute HTTP is restricted to exact loopback in debug
/// development; all other absolute endpoints must use HTTPS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderboardApiBaseUrl(String);

impl LeaderboardApiBaseUrl {
    pub fn production() -> Self {
        #[cfg(target_arch = "wasm32")]
        let value = SAME_ORIGIN_API_BASE_PATH;
        #[cfg(not(target_arch = "wasm32"))]
        let value = NATIVE_PRODUCTION_API_BASE_URL;
        Self(value.to_owned())
    }

    pub fn parse_development(raw: &str) -> Result<Self, LeaderboardPreferencesError> {
        normalize_api_base_url(raw, true).map(Self)
    }

    pub fn parse_production(raw: &str) -> Result<Self, LeaderboardPreferencesError> {
        normalize_api_base_url(raw, false).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn route(&self, suffix: &str) -> Result<String, LeaderboardPreferencesError> {
        if suffix.is_empty()
            || suffix.starts_with('/')
            || suffix.contains('#')
            || suffix.chars().any(char::is_control)
        {
            return Err(LeaderboardPreferencesError::InvalidRoute);
        }
        Ok(format!("{}/{suffix}", self.0))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LeaderboardPreferencesError {
    #[error("unsupported leaderboard-preferences schema {found}; expected {expected}")]
    UnsupportedSchema { found: u16, expected: u16 },
    #[error("leaderboard API URL must be the same-origin /api/v1 path or an absolute URL")]
    UnsupportedApiUrl,
    #[error("leaderboard API URL must use HTTPS (debug HTTP is exact-loopback only)")]
    InsecureApiUrl,
    #[error("leaderboard API URL must not contain credentials")]
    CredentialsInApiUrl,
    #[error("leaderboard API URL must not contain a query or fragment")]
    QueryOrFragmentInApiUrl,
    #[error("leaderboard API route is invalid")]
    InvalidRoute,
    #[error("leaderboard API environment override is invalid: {0}")]
    Environment(String),
    #[error("leaderboard preference {field} is empty or exceeds 128 bytes")]
    InvalidFacet { field: &'static str },
    #[error("leaderboard player-count filter is outside the supported seat range")]
    InvalidPlayerCount,
    #[error(transparent)]
    Storage(#[from] super::store::StoreError),
    #[error("failed to decode leaderboard preferences from {path}: {source}")]
    Decode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

const fn default_preferred_player_count() -> Option<u16> {
    Some(1)
}

const fn default_true() -> bool {
    true
}

fn validate_optional_facet(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), LeaderboardPreferencesError> {
    if value.is_some_and(|value| {
        value.is_empty() || value.len() > 128 || value.chars().any(char::is_control)
    }) {
        Err(LeaderboardPreferencesError::InvalidFacet { field })
    } else {
        Ok(())
    }
}

fn normalize_api_base_url(
    raw: &str,
    allow_loopback_http: bool,
) -> Result<String, LeaderboardPreferencesError> {
    let trimmed = raw.trim();
    if trimmed == SAME_ORIGIN_API_BASE_PATH {
        return Ok(trimmed.to_owned());
    }
    if trimmed.starts_with('/') {
        return Err(LeaderboardPreferencesError::UnsupportedApiUrl);
    }
    let parsed =
        url::Url::parse(trimmed).map_err(|_| LeaderboardPreferencesError::UnsupportedApiUrl)?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(LeaderboardPreferencesError::CredentialsInApiUrl);
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(LeaderboardPreferencesError::QueryOrFragmentInApiUrl);
    }
    match parsed.scheme() {
        "https" => {}
        "http" if allow_loopback_http && is_exact_loopback(&parsed) => {}
        "http" => return Err(LeaderboardPreferencesError::InsecureApiUrl),
        _ => return Err(LeaderboardPreferencesError::UnsupportedApiUrl),
    }
    let mut normalized = parsed.to_string();
    while normalized.ends_with('/') {
        normalized.pop();
    }
    Ok(normalized)
}

fn is_exact_loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(address)) => address == std::net::Ipv4Addr::LOCALHOST,
        Some(url::Host::Ipv6(address)) => address == std::net::Ipv6Addr::LOCALHOST,
        _ => false,
    }
}

pub fn load() -> Result<LeaderboardPreferences, LeaderboardPreferencesError> {
    let Some(encoded) = read_store()? else {
        return Ok(LeaderboardPreferences::default());
    };
    let preferences =
        serde_json::from_str::<LeaderboardPreferences>(&encoded).map_err(|source| {
            LeaderboardPreferencesError::Decode {
                path: display_path(),
                source,
            }
        })?;
    preferences.validate()?;
    Ok(preferences)
}

pub fn persist(preferences: &LeaderboardPreferences) -> Result<(), LeaderboardPreferencesError> {
    preferences.validate()?;
    let encoded = serde_json::to_vec_pretty(preferences)
        .expect("LeaderboardPreferences serialization cannot fail");
    persist_store(&encoded)
}

fn display_path() -> PathBuf {
    super::store::display_path(PREFERENCES_FILE, BROWSER_PREFERENCES_KEY)
}

fn read_store() -> Result<Option<String>, LeaderboardPreferencesError> {
    Ok(super::store::read(
        PREFERENCES_FILE,
        BROWSER_PREFERENCES_KEY,
    )?)
}
fn persist_store(encoded: &[u8]) -> Result<(), LeaderboardPreferencesError> {
    Ok(super::store::write(
        PREFERENCES_FILE,
        BROWSER_PREFERENCES_KEY,
        ".leaderboards-",
        encoded,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_submission_is_explicit_and_won_eligible_only() {
        let mut preferences = LeaderboardPreferences::default();
        assert!(preferences.show_mission_end_boards);
        assert!(!preferences.automatically_submit(true));
        preferences.always_submit_eligible_runs = true;
        assert!(preferences.automatically_submit(true));
        assert!(!preferences.automatically_submit(false));
    }

    #[test]
    fn production_is_fixed_and_development_url_policy_is_strict() {
        assert_eq!(
            LeaderboardApiBaseUrl::parse_production(SAME_ORIGIN_API_BASE_PATH)
                .unwrap()
                .as_str(),
            SAME_ORIGIN_API_BASE_PATH
        );
        assert_eq!(
            LeaderboardApiBaseUrl::parse_production("https://robinhood.phiresky.xyz/api/v1///")
                .unwrap()
                .as_str(),
            NATIVE_PRODUCTION_API_BASE_URL
        );
        assert!(LeaderboardApiBaseUrl::parse_production("/other").is_err());
        assert!(LeaderboardApiBaseUrl::parse_production("scores.example.test").is_err());
        assert!(LeaderboardApiBaseUrl::parse_production("file:///tmp/scores").is_err());
        assert!(
            LeaderboardApiBaseUrl::parse_production("https://user@example.test/api/v1").is_err()
        );
        assert!(
            LeaderboardApiBaseUrl::parse_production("https://example.test/api/v1?q=secret")
                .is_err()
        );
        assert!(LeaderboardApiBaseUrl::parse_production("http://localhost:8787/api/v1").is_err());
        assert!(LeaderboardApiBaseUrl::parse_development("http://localhost.evil/api/v1").is_err());
        assert!(LeaderboardApiBaseUrl::parse_development("http://127.0.0.2/api/v1").is_err());
        assert_eq!(
            LeaderboardApiBaseUrl::parse_development("http://[::1]:8787/api/v1/")
                .unwrap()
                .as_str(),
            "http://[::1]:8787/api/v1"
        );
    }

    #[test]
    fn routes_cannot_replace_the_validated_base() {
        let base = LeaderboardApiBaseUrl::parse_production(SAME_ORIGIN_API_BASE_PATH).unwrap();
        assert_eq!(
            base.route("leaderboard-metadata").unwrap(),
            "/api/v1/leaderboard-metadata"
        );
        assert!(base.route("/attacker.example").is_err());
        assert!(base.route("runs/ok#fragment").is_err());
    }

    #[test]
    fn validation_borrows_preferences_and_rejects_every_invalid_facet() {
        let valid = LeaderboardPreferences {
            preferred_preset_id: Some("custom".into()),
            preferred_difficulty_id: Some("hard".into()),
            preferred_competition_id: Some("season".into()),
            ..LeaderboardPreferences::default()
        };
        let before = serde_json::to_value(&valid).unwrap();
        valid.validate().unwrap();
        valid.validate().unwrap();
        assert_eq!(serde_json::to_value(&valid).unwrap(), before);
        for field in [
            "preferred_preset_id",
            "preferred_difficulty_id",
            "preferred_competition_id",
        ] {
            for invalid in [String::new(), "x".repeat(129), "invalid\nfacet".into()] {
                let mut document = before.clone();
                document[field] = invalid.into();
                let preferences: LeaderboardPreferences =
                    serde_json::from_value(document.clone()).unwrap();
                assert!(matches!(preferences.validate(),
                    Err(LeaderboardPreferencesError::InvalidFacet { field: actual }) if actual == field));
                assert_eq!(serde_json::to_value(&preferences).unwrap(), document);
            }
        }
        for count in [0, robin_run_protocol::MAX_REPLAY_SEATS_V1 + 1] {
            let invalid = LeaderboardPreferences {
                preferred_max_concurrent_players: Some(count),
                ..valid.clone()
            };
            assert!(matches!(
                invalid.validate(),
                Err(LeaderboardPreferencesError::InvalidPlayerCount)
            ));
        }
    }

    #[test]
    fn unknown_fields_and_future_schemas_fail_closed() {
        let unknown = r#"{
            "schema_version": 1,
            "always_submit_eligible_runs": false,
            "upload_without_consent": true
        }"#;
        assert!(serde_json::from_str::<LeaderboardPreferences>(unknown).is_err());

        let future = LeaderboardPreferences {
            schema_version: 2,
            ..LeaderboardPreferences::default()
        };
        assert!(future.validate().is_err());
    }

    #[test]
    fn older_preferences_default_only_the_presentation_toggle_on() {
        let decoded: LeaderboardPreferences = serde_json::from_str(
            r#"{
                "schema_version": 1
            }"#,
        )
        .unwrap();
        assert!(decoded.show_mission_end_boards);
        assert!(!decoded.always_submit_eligible_runs);
    }
}
