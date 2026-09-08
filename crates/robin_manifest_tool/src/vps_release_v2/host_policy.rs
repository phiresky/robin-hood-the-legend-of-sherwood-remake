//! Pure reviewed host-template policy and exact deployment inventories.
//!
//! This module receives one bounded byte snapshot. It neither opens files nor
//! deploys anything; callers retain responsibility for descriptor/path authority.

use super::{INSTALL_ROOT, VpsHostFileRoleV2, reject_placeholders};
use anyhow::{Context as _, Result, ensure};

pub(super) const SYSTEM_UNIT_ROOT: &str = "/etc/systemd/system";
pub(super) const CANONICAL_VALIDATE_RELEASE_SCRIPT: &str =
    include_str!("../../../robin_highscores/deploy/validate-release-bundle.sh");
pub(super) const CANONICAL_DEPLOY_RELEASE_SCRIPT: &str =
    include_str!("../../../robin_highscores/deploy/deploy-release.sh");
pub(super) const CANONICAL_ROLLBACK_RELEASE_SCRIPT: &str =
    include_str!("../../../robin_highscores/deploy/rollback-release.sh");
pub(super) const CANONICAL_REAL_FENCE_RELEASE_GATE: &str =
    include_str!("../../../robin_highscores/deploy/tests/real-runtime-fence-release-gate.sh");
pub(super) const CANONICAL_REAL_FENCE_HARNESS: &str =
    include_str!("../../../robin_highscores/deploy/tests/real-runtime-fence-e2e.py");
pub(super) const CANONICAL_REAL_FENCE_SELFTEST: &str =
    include_str!("../../../robin_highscores/deploy/tests/real-runtime-fence-e2e-selftest.py");
pub(super) const CANONICAL_ROOT_ONCE_SCRIPT: &str =
    include_str!("../../../robin_highscores/deploy/root-once.sh");
pub(super) const CANONICAL_NGINX_CHALLENGE: &str =
    include_str!("../../../robin_highscores/deploy/nginx-robinhood-api.challenge.conf");
pub(super) const CANONICAL_NGINX_CLOUDFLARE_ONLY: &str =
    include_str!("../../../robin_highscores/deploy/nginx-robinhood-cloudflare-only.conf");
pub(super) const CANONICAL_NGINX_API_LOCATIONS: &str =
    include_str!("../../../robin_highscores/deploy/nginx-robinhood-api.locations.conf");
pub(super) const CANONICAL_NGINX_VHOST: &str =
    include_str!("../../../robin_highscores/deploy/nginx-robinhood-api.vhost.conf");
pub(super) const CANONICAL_USER_TARGET: &str =
    include_str!("../../../robin_highscores/deploy/robin-highscores.target");
pub(super) const CANONICAL_API_SERVICE: &str =
    include_str!("../../../robin_highscores/deploy/robin-highscores-api.service");
pub(super) const CANONICAL_WORKER_SERVICE: &str =
    include_str!("../../../robin_highscores/deploy/robin-highscores-worker.service");
pub(super) const CANONICAL_BACKUP_SERVICE: &str =
    include_str!("../../../robin_highscores/deploy/robin-highscores-backup.service");
pub(super) const CANONICAL_BACKUP_TIMER: &str =
    include_str!("../../../robin_highscores/deploy/robin-highscores-backup.timer");
pub(super) const VALIDATOR_SYSTEM_UNIT_DENYLIST_BLOCK: &str = concat!(
    "    unit_path=$bundle/systemd/user/$unit\n",
    "    grep -Fq \"$release_root\" \"$unit_path\" || fail \"$unit is not pinned to the exact release\"\n",
    "    if grep -Eq '^(User|Group|SupplementaryGroups)=|WantedBy=multi-user.target|/etc/systemd/system|verifier-broker|sudo|polkit' \"$unit_path\"; then\n",
    "        fail \"$unit contains a root/system-service assumption\"\n",
    "    fi\n",
);
pub(super) const ROOT_ONCE_KIT_FILES: [&str; 5] = [
    "nginx-robinhood-api.challenge.conf",
    "nginx-robinhood-cloudflare-only.conf",
    "nginx-robinhood-api.locations.conf",
    "nginx-robinhood-api.vhost.conf",
    "root-once.sh",
];
pub(super) const DEPLOY_BOOTSTRAP_FILES: [&str; 3] = [
    "deploy-release.sh",
    "rollback-release.sh",
    "validate-release-bundle.sh",
];

pub(super) fn validate_host_template_bytes(
    role: VpsHostFileRoleV2,
    bytes: &[u8],
    source_commit: &str,
) -> Result<()> {
    let text = std::str::from_utf8(bytes).context("host deployment file is not UTF-8")?;
    reject_placeholders(bytes, "host deployment file")?;
    let release_root = format!("{INSTALL_ROOT}/releases/{source_commit}");
    validate_system_unit_root_authority(role, text)?;
    let canonical_bytes = match role {
        VpsHostFileRoleV2::UserTarget => Some(CANONICAL_USER_TARGET.to_owned()),
        VpsHostFileRoleV2::ApiService => {
            Some(CANONICAL_API_SERVICE.replace("@SOURCE_COMMIT@", source_commit))
        }
        VpsHostFileRoleV2::WorkerService => {
            Some(CANONICAL_WORKER_SERVICE.replace("@SOURCE_COMMIT@", source_commit))
        }
        VpsHostFileRoleV2::BackupService => {
            Some(CANONICAL_BACKUP_SERVICE.replace("@SOURCE_COMMIT@", source_commit))
        }
        VpsHostFileRoleV2::BackupTimer => Some(CANONICAL_BACKUP_TIMER.to_owned()),
        VpsHostFileRoleV2::DeployReleaseScript => Some(CANONICAL_DEPLOY_RELEASE_SCRIPT.to_owned()),
        VpsHostFileRoleV2::RollbackReleaseScript => {
            Some(CANONICAL_ROLLBACK_RELEASE_SCRIPT.to_owned())
        }
        VpsHostFileRoleV2::ValidateReleaseScript => {
            Some(CANONICAL_VALIDATE_RELEASE_SCRIPT.to_owned())
        }
        VpsHostFileRoleV2::RealRuntimeFenceReleaseGate => {
            Some(CANONICAL_REAL_FENCE_RELEASE_GATE.to_owned())
        }
        VpsHostFileRoleV2::RealRuntimeFenceHarness => Some(CANONICAL_REAL_FENCE_HARNESS.to_owned()),
        VpsHostFileRoleV2::RealRuntimeFenceSelftest => {
            Some(CANONICAL_REAL_FENCE_SELFTEST.to_owned())
        }
        VpsHostFileRoleV2::RootOnceScript => Some(CANONICAL_ROOT_ONCE_SCRIPT.to_owned()),
        VpsHostFileRoleV2::NginxChallenge => Some(CANONICAL_NGINX_CHALLENGE.to_owned()),
        VpsHostFileRoleV2::NginxCloudflareOnly => Some(CANONICAL_NGINX_CLOUDFLARE_ONLY.to_owned()),
        VpsHostFileRoleV2::NginxApiLocations => Some(CANONICAL_NGINX_API_LOCATIONS.to_owned()),
        VpsHostFileRoleV2::NginxVhost => Some(CANONICAL_NGINX_VHOST.to_owned()),
        _ => None,
    };
    if let Some(canonical) = canonical_bytes {
        ensure!(
            text == canonical,
            "security-sensitive host file differs from its exact canonical repository template"
        );
    } else {
        reject_placeholders(bytes, "host deployment file")?;
    }
    match role {
        VpsHostFileRoleV2::UserTarget => {
            ensure!(
                text.contains("robin-highscores-api.service")
                    && text.contains("robin-highscores-worker.service")
                    && text.contains("WantedBy=default.target"),
                "user target does not own the API/worker boot lifecycle"
            );
            validate_user_unit(text)?;
        }
        VpsHostFileRoleV2::ApiService => {
            ensure!(
                text.contains(&format!(
                    "ExecStart={release_root}/bin/robin-highscores-server --config {release_root}/config/highscores-server.toml"
                )),
                "API user unit is not pinned to the exact release"
            );
            validate_user_unit(text)?;
        }
        VpsHostFileRoleV2::WorkerService => {
            ensure!(
                text.contains(&format!(
                    "ExecStart={release_root}/bin/robin-highscores-worker --config {release_root}/config/highscores-worker.toml"
                )),
                "worker user unit is not pinned to the exact release"
            );
            validate_user_unit(text)?;
        }
        VpsHostFileRoleV2::BackupService => {
            ensure!(
                text.contains(&format!(
                    "ExecStart={release_root}/bin/robin-highscores-admin"
                )) && text.contains(&format!(
                    "--config {release_root}/config/highscores-server.toml"
                )),
                "backup user unit is not pinned to the exact release"
            );
            validate_user_unit(text)?;
        }
        VpsHostFileRoleV2::BackupTimer => {
            ensure!(
                text.contains("Unit=robin-highscores-backup.service")
                    && text.contains("WantedBy=timers.target"),
                "backup timer does not activate the user backup service"
            );
            validate_user_unit(text)?;
        }
        VpsHostFileRoleV2::DeployReleaseScript | VpsHostFileRoleV2::RollbackReleaseScript => {
            validate_literal_install_root_assignment(text)?;
            ensure!(
                text.starts_with("#!/bin/sh\n")
                    && text.contains(INSTALL_ROOT)
                    && text.contains("systemctl --user"),
                "user release script does not use the canonical install root and user manager"
            );
            ensure!(
                !text.contains("sudo ") && !text.contains("/etc/systemd/system"),
                "user release script retains privileged activation authority"
            );
            if role == VpsHostFileRoleV2::DeployReleaseScript {
                ensure!(
                    text.contains(
                        "managed state directory is not exact and will not be repaired"
                    ) && text.contains(
                        "backup state directory is missing on upgrade and will not be repaired"
                    ) && text.contains("if [ \"$receipt_source_commit\" = none ]; then")
                        && text.contains(
                            "for initialized_directory in \"$backup_root\" \"$state_root/status\"; do"
                        )
                        && text.contains("mkdir -m 0700 -- \"$initialized_directory\"")
                        && text.contains(
                            "could not durably bind the initialized runtime authority"
                        )
                        && text.contains(
                            "deploy/tests/real-runtime-fence-release-gate.sh"
                        )
                        && text.contains("ROBIN_REAL_FENCE_PINNED_CANDIDATE_FD")
                        && text.contains(
                            "mandatory authentic runtime-fence release gate failed before activation mutation"
                        )
                        && !text.contains("chmod 0700 -- \"$state_root\"")
                        && !text.contains("chmod 0700 -- \"$managed_directory\""),
                    "deploy script must validate all upgrade state without repair and create clean-first backup state only after durable runtime authority"
                );
            }
        }
        VpsHostFileRoleV2::ValidateReleaseScript => {
            ensure!(
                text.starts_with("#!/bin/sh\n")
                    && text.contains("robin-highscores-manifestctl")
                    && text.contains("SHA256SUMS")
                    && text.contains("MODE_INVENTORY"),
                "release validator does not check the typed bundle and both inventories"
            );
            ensure!(
                !text.contains("sudo "),
                "release validator retains privileged activation authority"
            );
        }
        VpsHostFileRoleV2::RealRuntimeFenceReleaseGate
        | VpsHostFileRoleV2::RealRuntimeFenceHarness
        | VpsHostFileRoleV2::RealRuntimeFenceSelftest => {
            ensure!(
                text.starts_with("#!/bin/sh\n") || text.starts_with("#!/usr/bin/env python3\n"),
                "real runtime-fence gate authority has no exact interpreter"
            );
            ensure!(
                !text.contains("sudo ") && !text.contains("/etc/systemd/system"),
                "real runtime-fence gate retains privileged mutation authority"
            );
        }
        VpsHostFileRoleV2::RootOnceScript => {
            ensure!(
                text.starts_with("#!/bin/sh\n") && text.contains("nginx"),
                "root-once script is not the reviewed nginx setup"
            );
            ensure!(
                !text.contains("/etc/systemd/system") && !text.contains("useradd"),
                "root-once nginx script retains service/principal bootstrap authority"
            );
        }
        VpsHostFileRoleV2::NginxChallenge => {
            ensure!(
                text.contains(".well-known/acme-challenge"),
                "nginx challenge vhost omits the ACME challenge route"
            );
        }
        VpsHostFileRoleV2::NginxCloudflareOnly => {
            ensure!(
                text.contains("allow ") && text.contains("deny all"),
                "nginx Cloudflare include is not a fail-closed allowlist"
            );
        }
        VpsHostFileRoleV2::NginxApiLocations => {
            ensure!(
                text.contains("127.0.0.1:8787") && text.contains("/api"),
                "nginx include does not route the loopback leaderboard API"
            );
        }
        VpsHostFileRoleV2::NginxVhost => {
            ensure!(
                text.contains("robinhood.phiresky.xyz")
                    && text.contains("robinhood-api.locations.conf"),
                "nginx vhost does not bind the production domain and API include"
            );
        }
        VpsHostFileRoleV2::DeploymentReadme
        | VpsHostFileRoleV2::OperatorRunbook
        | VpsHostFileRoleV2::BackupRunbook => {}
    }
    Ok(())
}

pub(super) fn validate_system_unit_root_authority(
    role: VpsHostFileRoleV2,
    text: &str,
) -> Result<()> {
    if role == VpsHostFileRoleV2::ValidateReleaseScript {
        ensure!(
            text == CANONICAL_VALIDATE_RELEASE_SCRIPT
                && text.matches(SYSTEM_UNIT_ROOT).count() == 1
                && text.matches(VALIDATOR_SYSTEM_UNIT_DENYLIST_BLOCK).count() == 1,
            "release validator must be the exact canonical script with one system-unit-root denylist block"
        );
    } else {
        ensure!(
            !text.contains(SYSTEM_UNIT_ROOT),
            "host deployment file retains root system-service authority"
        );
    }
    Ok(())
}

pub(super) fn validate_literal_install_root_assignment(text: &str) -> Result<()> {
    let expected = format!("opt_root={INSTALL_ROOT}");
    let mut assignments = text
        .lines()
        .filter(|line| line.trim_start().starts_with("opt_root="));
    ensure!(
        assignments.next() == Some(expected.as_str()) && assignments.next().is_none(),
        "user release script must contain exactly one literal {expected} assignment"
    );
    Ok(())
}

fn validate_user_unit(text: &str) -> Result<()> {
    for forbidden in [
        "User=",
        "Group=",
        "SupplementaryGroups=",
        "WantedBy=multi-user.target",
        "/etc/systemd/system",
        "/var/lib/robin-highscores",
        "/srv/robin-highscores",
        "verifier-broker",
        "systemd-run",
        "polkit",
    ] {
        ensure!(
            !text.contains(forbidden),
            "user unit retains obsolete root deployment field {forbidden}"
        );
    }
    ensure!(
        !text
            .split(|character: char| character.is_ascii_whitespace() || character == '=')
            .any(|token| token.starts_with("/opt/robin-highscores")),
        "user unit retains the obsolete root-owned /opt release path"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_validates_exact_bytes_without_filesystem_inputs() -> Result<()> {
        let commit = "a".repeat(40);
        for (role, template) in [
            (VpsHostFileRoleV2::ApiService, CANONICAL_API_SERVICE),
            (VpsHostFileRoleV2::WorkerService, CANONICAL_WORKER_SERVICE),
            (VpsHostFileRoleV2::BackupService, CANONICAL_BACKUP_SERVICE),
            (
                VpsHostFileRoleV2::DeployReleaseScript,
                CANONICAL_DEPLOY_RELEASE_SCRIPT,
            ),
            (
                VpsHostFileRoleV2::RollbackReleaseScript,
                CANONICAL_ROLLBACK_RELEASE_SCRIPT,
            ),
            (
                VpsHostFileRoleV2::ValidateReleaseScript,
                CANONICAL_VALIDATE_RELEASE_SCRIPT,
            ),
        ] {
            let exact = template.replace("@SOURCE_COMMIT@", &commit);
            validate_host_template_bytes(role, exact.as_bytes(), &commit)?;
            let changed = format!("{exact}\n# changed authority\n");
            assert!(validate_host_template_bytes(role, changed.as_bytes(), &commit).is_err());
        }
        Ok(())
    }

    #[test]
    fn policy_rejects_invalid_text_and_wrong_release_commit() {
        assert!(
            validate_host_template_bytes(VpsHostFileRoleV2::DeploymentReadme, &[0xff], "unused")
                .is_err()
        );
        let rendered = CANONICAL_API_SERVICE.replace("@SOURCE_COMMIT@", &"a".repeat(40));
        assert!(
            validate_host_template_bytes(
                VpsHostFileRoleV2::ApiService,
                rendered.as_bytes(),
                &"b".repeat(40)
            )
            .is_err()
        );
        assert!(
            validate_host_template_bytes(
                VpsHostFileRoleV2::ApiService,
                CANONICAL_API_SERVICE.as_bytes(),
                &"a".repeat(40)
            )
            .is_err()
        );
    }
}
