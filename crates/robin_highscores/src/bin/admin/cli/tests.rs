use super::*;

#[test]
fn ops_commands_parse() {
    let arguments = Arguments::try_parse_from([
        "robin-highscores-admin",
        "--config",
        "/etc/server.toml",
        "snapshot-db",
        "/backups/highscores.sqlite3",
    ])
    .unwrap();
    assert!(matches!(
        arguments.command,
        Command::SnapshotDb { path } if path == Path::new("/backups/highscores.sqlite3")
    ));
    assert!(Arguments::try_parse_from(["robin-highscores-admin", "snapshot-db"]).is_err());
    assert!(matches!(
        Arguments::try_parse_from(["robin-highscores-admin", "database-schema-version"])
            .unwrap()
            .command,
        Command::DatabaseSchemaVersion
    ));
    assert!(matches!(
        Arguments::try_parse_from(["robin-highscores-admin", "supported-schema-version"])
            .unwrap()
            .command,
        Command::SupportedSchemaVersion
    ));
    for removed in [
        "backup-and-publish-status",
        "verify-backup",
        "probe-runtime-authority-v2",
        "verify-live-database-schema-v2",
        "initialize-backup-authority-key-v2",
    ] {
        assert!(Arguments::try_parse_from(["robin-highscores-admin", removed]).is_err());
    }
}

#[test]
fn all_secret_bootstraps_load_only_the_requested_path() {
    let directory = tempfile::tempdir().unwrap();
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
    assert!(load_secret_bootstrap_config(&config_path, "moderation_bearer_token_path").is_err());
}
