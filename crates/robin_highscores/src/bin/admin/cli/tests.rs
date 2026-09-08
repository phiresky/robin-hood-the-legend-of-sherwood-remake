use super::super::fixtures::test_release_identity;
use super::*;
use robin_highscores::runtime_authority::BackupAuthorityStateV2;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

#[test]
fn backup_authority_key_v2_cli_has_no_configurable_authority_paths() {
    let initialize = Arguments::try_parse_from([
        "admin",
        "initialize-backup-authority-key-v2",
        "--source-commit",
        "0123456789abcdef0123456789abcdef01234567",
        "--activation-lock-fd",
        "9",
    ])
    .unwrap();
    assert!(matches!(
        initialize.command,
        Command::InitializeBackupAuthorityKeyV2 {
            activation_lock_fd: 9,
            ..
        }
    ));
    assert!(
        Arguments::try_parse_from([
            "admin",
            "initialize-backup-authority-key-v2",
            "--source-commit",
            "0123456789abcdef0123456789abcdef01234567",
            "--activation-lock-fd",
            "9",
            "--key-path",
            "/tmp/injected",
        ])
        .is_err()
    );
    assert!(
        Arguments::try_parse_from([
            "admin",
            "complete-backup-authority-key-v2",
            "--source-commit",
            "0123456789abcdef0123456789abcdef01234567",
            "--activation-lock-fd",
            "9",
            "--candidate-release-root-fd",
            "10",
            "--expected-vps-release-manifest-sha256",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ])
        .is_ok()
    );
}

#[test]
fn runtime_authority_probe_has_an_exact_config_free_cli_contract() {
    let command = Arguments::try_parse_from([
        "robin-highscores-admin",
        "--config",
        "/definitely/not/a/runtime-authority-input.toml",
        "probe-runtime-authority-v2",
        "--candidate-release-root-fd",
        "7",
        "--expected-vps-release-manifest-sha256",
        "abababababababababababababababababababababababababababababababab",
        "--backup-authority-state",
        "present",
    ])
    .unwrap()
    .command;
    assert!(matches!(
        command,
        Command::ProbeRuntimeAuthorityV2 {
            candidate_release_root_fd: 7,
            backup_authority_state: BackupAuthorityStateV2::Present,
            ..
        }
    ));
    assert!(
        Arguments::try_parse_from([
            "robin-highscores-admin",
            "probe-runtime-authority-v2",
            "--candidate-release-root-fd",
            "7",
            "--expected-vps-release-manifest-sha256",
            "abababababababababababababababababababababababababababababababab",
            "--backup-authority-state",
            "present",
            "--ambient-config-override",
            "/tmp/not-allowed",
        ])
        .is_err(),
        "the probe must not acquire an ambient override surface"
    );
}

#[test]
fn transaction_and_offline_verifier_modes_have_distinct_typed_cli_contracts() {
    let identity = test_release_identity();
    let common = [
        "--backup-authority-key-fd",
        "6",
        "--expected-source-commit",
        identity.source_commit.as_str(),
        "--expected-vps-release-manifest-sha256",
        identity.vps_release_manifest_sha256.as_str(),
        "--expected-publication-lock-sha256",
        identity.publication_lock_sha256.as_str(),
    ];
    let mut offline = vec!["robin-highscores-admin", "verify-backup"];
    offline.extend(["--backup-root-fd", "4"]);
    offline.extend(["--backup-directory-fd", "3"]);
    offline.extend(common);
    offline.extend([
        "--expected-backup-manifest-sha256",
        "abababababababababababababababababababababababababababababababab",
    ]);
    assert!(matches!(
        Arguments::try_parse_from(offline).unwrap().command,
        Command::VerifyBackup { .. }
    ));

    let mut transaction = vec!["robin-highscores-admin", "verify-transaction-backup"];
    transaction.extend(["--backup-root-fd", "3"]);
    transaction.extend(["--status-envelope-fd", "4"]);
    transaction.extend(["--expected-release-manifest-fd", "5"]);
    transaction.extend(common);
    assert!(matches!(
        Arguments::try_parse_from(transaction.clone())
            .unwrap()
            .command,
        Command::VerifyTransactionBackup { .. }
    ));
    transaction.extend([
        "--expected-backup-manifest-sha256",
        "abababababababababababababababababababababababababababababababab",
    ]);
    assert!(Arguments::try_parse_from(transaction).is_err());

    let mut shell_selected_child = vec!["robin-highscores-admin", "verify-transaction-backup"];
    shell_selected_child.extend(["--backup-directory-fd", "3"]);
    shell_selected_child.extend(["--status-envelope-fd", "4"]);
    shell_selected_child.extend(["--expected-release-manifest-fd", "5"]);
    shell_selected_child.extend(common);
    assert!(
        Arguments::try_parse_from(shell_selected_child).is_err(),
        "transaction callers may pass only the canonical root FD, never a shell-selected child"
    );

    let mut missing_offline_digest = vec!["robin-highscores-admin", "verify-backup"];
    missing_offline_digest.extend(["--backup-root-fd", "4"]);
    missing_offline_digest.extend(["--backup-directory-fd", "3"]);
    missing_offline_digest.extend(common);
    assert!(Arguments::try_parse_from(missing_offline_digest).is_err());
}

#[test]
fn live_schema_probe_has_one_config_free_typed_cli_contract() {
    let digest = "12".repeat(32);
    let arguments = Arguments::try_parse_from([
        "robin-highscores-admin",
        "verify-live-database-schema-v2",
        "--candidate-release-root-fd",
        "3",
        "--expected-vps-release-manifest-sha256",
        digest.as_str(),
    ])
    .unwrap();
    assert!(matches!(
        arguments.command,
        Command::VerifyLiveDatabaseSchemaV2 {
            candidate_release_root_fd: 3,
            expected_vps_release_manifest_sha256,
        } if expected_vps_release_manifest_sha256 == digest
    ));
    assert!(
        Arguments::try_parse_from([
            "robin-highscores-admin",
            "verify-live-database-schema-v2",
            "--candidate-release-root-fd",
            "3",
        ])
        .is_err()
    );
    assert!(
        Arguments::try_parse_from([
            "robin-highscores-admin",
            "verify-live-database-schema-v2",
            "--candidate-release-root-fd",
            "3",
            "--expected-vps-release-manifest-sha256",
            digest.as_str(),
            "--database-path",
            "/tmp/substituted.sqlite3",
        ])
        .is_err(),
        "production live-schema verification must not accept a path override"
    );
}

#[test]
fn all_secret_bootstraps_load_only_the_requested_path() {
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let cursor = directory.path().join("cursor.key");
    let competition = directory.path().join("competition.key");
    let preflight = directory.path().join("preflight.key");
    let config_path = directory.path().join("bootstrap.toml");
    std::fs::write(
        &config_path,
        format!(
            "cursor_secret_path = \"{}\"\ncompetition_run_grant_secret_path = \"{}\"\nrun_preflight_grant_secret_path = \"{}\"\ndatabase_path = \"relative-and-invalid-for-final-config\"\n",
            cursor.display(),
            competition.display(),
            preflight.display(),
        ),
    )
    .unwrap();
    let config = load_secret_bootstrap_config(&config_path, "cursor_secret_path").unwrap();
    config.load_or_create_cursor_key().unwrap();
    assert_eq!(std::fs::read(&cursor).unwrap().len(), 32);
    let config =
        load_secret_bootstrap_config(&config_path, "competition_run_grant_secret_path").unwrap();
    assert_eq!(
        config
            .load_or_create_competition_run_grant_key()
            .unwrap()
            .len(),
        32
    );
    let config =
        load_secret_bootstrap_config(&config_path, "run_preflight_grant_secret_path").unwrap();
    assert_eq!(
        config
            .load_or_create_run_preflight_grant_key()
            .unwrap()
            .len(),
        32
    );
    assert_eq!(std::fs::read(competition).unwrap().len(), 32);
    assert_eq!(std::fs::read(preflight).unwrap().len(), 32);
    assert!(
        load_secret_bootstrap_config(&config_path, "backup_authority_hmac_secret_path").is_err(),
        "the non-resumable legacy fifth-key bootstrap must not be reachable"
    );
    assert!(load_secret_bootstrap_config(&config_path, "moderation_bearer_token_path").is_err());
}
