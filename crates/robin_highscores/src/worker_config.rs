//! `worker.toml`: queue policy, sandbox launcher, decoder limits and the raw
//! content root of each official edition.

use crate::verifier::VerifierLauncherConfig;
use robin_run_protocol::{OfficialContentEditionV1, Validate as _, VerificationLimitsV1};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub server_config: PathBuf,
    pub worker_id: String,
    pub poll_interval_ms: u64,
    pub lease_seconds: u64,
    pub retry_seconds: u64,
    pub max_verifier_attempts: u32,
    pub verifier_launcher: VerifierLauncherConfig,
    pub limits: VerificationLimitsV1,
    pub content: EditionContentRoots,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditionContentRoots {
    pub demo: EditionContent,
    pub full: EditionContent,
}

/// Operator-installed, read-only raw game content of one edition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditionContent {
    pub root: PathBuf,
    /// Single numeric locale directory below `root`, e.g. `1033` (Demo) or
    /// `2047` (Full).
    pub resource_locale_root: String,
}

impl EditionContentRoots {
    pub fn edition(&self, edition: OfficialContentEditionV1) -> &EditionContent {
        match edition {
            OfficialContentEditionV1::Demo => &self.demo,
            OfficialContentEditionV1::Full => &self.full,
        }
    }
}

impl WorkerConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let bytes = crate::secure_fs::read_bounded_no_symlinks(path, 1024 * 1024)?;
        let config: Self = toml::from_str(std::str::from_utf8(&bytes)?)?;
        config.validate()?;
        Ok(config)
    }

    /// Structural validation. Launcher programs and content directories are
    /// checked against the filesystem separately at worker startup.
    pub fn validate(&self) -> anyhow::Result<()> {
        let minimum_lease_seconds = self
            .verifier_launcher
            .wall_timeout_seconds
            .checked_add(30)
            .ok_or_else(|| anyhow::anyhow!("verifier timeout overflows the lease safety margin"))?;
        anyhow::ensure!(
            !self.worker_id.is_empty() && self.worker_id.len() <= 128,
            "worker_id must contain 1..=128 characters"
        );
        anyhow::ensure!(
            self.server_config.is_absolute(),
            "server_config must be an absolute path"
        );
        anyhow::ensure!(
            self.poll_interval_ms > 0
                && self.lease_seconds > minimum_lease_seconds
                && self.retry_seconds > 0
                && (1..=100).contains(&self.max_verifier_attempts),
            "worker polling, lease, retry, and verifier timeout are inconsistent"
        );
        self.limits.validate()?;
        for (name, content) in [("demo", &self.content.demo), ("full", &self.content.full)] {
            anyhow::ensure!(
                content.root.is_absolute()
                    && content.root.components().all(|component| matches!(
                        component,
                        Component::RootDir | Component::Normal(_)
                    ))
                    && content.root.file_name().is_some(),
                "content.{name}.root must be a normalized absolute path"
            );
            anyhow::ensure!(
                (1..=8).contains(&content.resource_locale_root.len())
                    && content
                        .resource_locale_root
                        .bytes()
                        .all(|byte| byte.is_ascii_digit()),
                "content.{name}.resource_locale_root must be 1..=8 decimal digits"
            );
        }
        anyhow::ensure!(
            self.content.demo.root != self.content.full.root,
            "Demo and Full content roots must be distinct"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_and_production_worker_configs_parse_with_content_roots() {
        for (name, text) in [
            ("example", include_str!("../highscores-worker.example.toml")),
            ("production", include_str!("../ops/production/worker.toml")),
        ] {
            let config: WorkerConfig =
                toml::from_str(text).unwrap_or_else(|error| panic!("{name}: {error}"));
            config
                .validate()
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(config.content.demo.resource_locale_root, "1033");
            assert_eq!(config.content.full.resource_locale_root, "2047");
            assert_eq!(
                config.content.edition(OfficialContentEditionV1::Full).root,
                Path::new("/home/robinhood/.local/share/robin-highscores/raw-content/full")
            );
        }
    }

    #[test]
    fn worker_config_rejects_unknown_fields_and_bad_content_roots() {
        let text = include_str!("../ops/production/worker.toml");
        let with_pin = text.replace(
            "[verifier_launcher]\n",
            "[verifier_launcher]\nverifier_sha256 = \"00\"\n",
        );
        assert!(toml::from_str::<WorkerConfig>(&with_pin).is_err());

        let mut config: WorkerConfig = toml::from_str(text).unwrap();
        config.content.full.resource_locale_root = "en-US".into();
        assert!(config.validate().is_err());
        let mut config: WorkerConfig = toml::from_str(text).unwrap();
        config.content.demo.root = "relative/demo".into();
        assert!(config.validate().is_err());
        let mut config: WorkerConfig = toml::from_str(text).unwrap();
        config.content.demo.root = config.content.full.root.clone();
        assert!(config.validate().is_err());
        let mut config: WorkerConfig = toml::from_str(text).unwrap();
        config.lease_seconds = config.verifier_launcher.wall_timeout_seconds;
        assert!(config.validate().is_err());
    }
}
