//! Shared test data constructors; no production authority or reexports.

use super::filesystem::record_file;
use super::filesystem::write_private_file;
use super::policy::SYSTEMD_UNIT_FILES;
use super::policy::SYSTEMD_USER_ROOT;
use robin_highscores::ServerConfig;
use robin_highscores::backup::BackupManifestV4 as BackupManifest;
use robin_highscores::backup::BackupReleaseIdentityV2;
use robin_highscores::backup::BackupVerificationEnvelopeV2;
#[cfg(test)]
use robin_highscores::backup::load_backup_release_identity_oob;
#[cfg(test)]
use robin_run_protocol::ArtifactRefV1;
#[cfg(test)]
use robin_run_protocol::Digest32;
use robin_run_protocol::canonical_json_bytes;
use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;

pub(super) fn test_release_identity() -> BackupReleaseIdentityV2 {
    BackupReleaseIdentityV2 {
        source_commit: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        database_schema_version: robin_run_protocol::HIGHSCORES_DATABASE_SCHEMA_VERSION,
        vps_release_manifest_sha256: "12".repeat(32),
        publication_lock_sha256: "34".repeat(32),
        installed_user_units: test_release_units(),
    }
}

pub(super) fn test_release_units() -> Vec<robin_highscores::backup::BackupReleaseUnitV2> {
    let mut units = SYSTEMD_UNIT_FILES
        .into_iter()
        .map(|unit| {
            let bytes = format!("fixture {unit}\n");
            robin_highscores::backup::BackupReleaseUnitV2 {
                release_relative_path: format!("systemd/user/{unit}"),
                artifact: ArtifactRefV1 {
                    sha256: Digest32::digest_bytes(bytes.as_bytes()),
                    byte_length: u64::try_from(bytes.len()).unwrap(),
                    media_type: "text/plain".to_owned(),
                },
                unix_mode: 0o440,
            }
        })
        .collect::<Vec<_>>();
    units.sort_by(|left, right| left.release_relative_path.cmp(&right.release_relative_path));
    units
}

pub(super) async fn write_test_release_manifest(path: &Path) -> BackupReleaseIdentityV2 {
    let mut files = vec![serde_json::json!({
        "artifact": {
            "byte_length": 1,
            "media_type": "application/octet-stream",
            "sha256": "9a".repeat(32),
        },
        "path": "README.md",
        "unix_mode": 0o440,
    })];
    files.extend(test_release_units().into_iter().map(|unit| {
        serde_json::json!({
            "artifact": unit.artifact,
            "path": unit.release_relative_path,
            "unix_mode": unit.unix_mode,
        })
    }));
    let document = serde_json::json!({
        "database_schema_version": robin_run_protocol::HIGHSCORES_DATABASE_SCHEMA_VERSION,
        "deployment": {
            "current_link": "/home/robinhood/.local/opt/robin-highscores/current",
            "home": "/home/robinhood",
            "install_root": "/home/robinhood/.local/opt/robin-highscores",
            "persistent_state_root": "/home/robinhood/.local/share/robin-highscores",
            "user": "robinhood",
        },
        "files": files,
        "publication_lock_sha256": "34".repeat(32),
        "publication_manifest_sha256": "56".repeat(32),
        "schema_version": 2,
        "source_commit": "0123456789abcdef0123456789abcdef01234567",
        "verifier_sha256": "78".repeat(32),
    });
    write_private_file(
        path,
        &robin_run_protocol::canonical_json_bytes(&document).unwrap(),
    )
    .await
    .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o440)).unwrap();
    load_backup_release_identity_oob(path).await.unwrap()
}

pub(super) async fn test_restore_sources(
    config: &ServerConfig,
    root: &Path,
) -> BTreeMap<PathBuf, PathBuf> {
    let mut sources = BTreeMap::new();
    for secret in [
        &config.cursor_secret_path,
        &config.competition_run_grant_secret_path,
        &config.run_preflight_grant_secret_path,
        config.moderation_bearer_token_path.as_ref().unwrap(),
    ] {
        #[cfg(unix)]
        std::fs::set_permissions(secret, std::fs::Permissions::from_mode(0o400)).unwrap();
        sources.insert(secret.clone(), secret.clone());
    }
    let units = root.join("installed-user-units");
    tokio::fs::create_dir(&units).await.unwrap();
    for unit in SYSTEMD_UNIT_FILES {
        let source = units.join(unit);
        write_private_file(&source, format!("fixture {unit}\n").as_bytes())
            .await
            .unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o440)).unwrap();
        sources.insert(Path::new(SYSTEMD_USER_ROOT).join(unit), source);
    }
    sources
}

pub(super) async fn refresh_database_manifest_entry(directory: &Path) {
    let manifest_path = directory.join("backup-manifest.json");
    let mut manifest: BackupManifest =
        serde_json::from_slice(&tokio::fs::read(&manifest_path).await.unwrap()).unwrap();
    let database = record_file(directory, &directory.join("highscores.sqlite3"))
        .await
        .unwrap();
    *manifest
        .files
        .iter_mut()
        .find(|entry| entry.relative_path == "highscores.sqlite3")
        .unwrap() = database;
    tokio::fs::write(&manifest_path, canonical_json_bytes(&manifest).unwrap())
        .await
        .unwrap();
    let backup_id = directory.file_name().unwrap().to_str().unwrap().to_owned();
    let envelope =
        BackupVerificationEnvelopeV2::new_authenticated(backup_id, &manifest, &[0x31; 32]).unwrap();
    let envelope_path = directory.join("backup-verification-envelope.json");
    #[cfg(unix)]
    std::fs::set_permissions(&envelope_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    tokio::fs::write(&envelope_path, canonical_json_bytes(&envelope).unwrap())
        .await
        .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&envelope_path, std::fs::Permissions::from_mode(0o400)).unwrap();
}

pub(super) async fn remove_test_database_sidecars(database: &Path) {
    for suffix in ["-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{suffix}", database.display()));
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to remove test SQLite sidecar: {error}"),
        }
    }
}
