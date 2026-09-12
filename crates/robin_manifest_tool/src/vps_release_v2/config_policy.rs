//! config policy responsibilities of the admitted release pipeline.
use super::*;

pub(super) fn validate_final_config(
    role: VpsConfigRoleV2,
    path: &Path,
    verifier_sha256: Digest32,
    job_catalog: &ArtifactRefV1,
    source_commit: &str,
    campaign_states: &BTreeSet<Digest32>,
) -> Result<()> {
    let bytes = read_regular_file_bounded(path, MAX_CONFIG_BYTES)?;
    reject_placeholders(&bytes, "release config")?;
    match role {
        VpsConfigRoleV2::ApiEnvironment | VpsConfigRoleV2::WorkerEnvironment => {
            ensure!(
                bytes == b"RUST_LOG=info\n",
                "release environment files may contain only RUST_LOG=info"
            );
        }
        VpsConfigRoleV2::Server => {
            let value: toml::Value = toml::from_str(std::str::from_utf8(&bytes)?)?;
            reject_obsolete_toml(&value)?;
            for (field, expected) in [
                ("bind", "127.0.0.1:8787"),
                (
                    "database_path",
                    "/home/robinhood/.local/share/robin-highscores/database/highscores.sqlite3",
                ),
                (
                    "replay_directory",
                    "/home/robinhood/.local/share/robin-highscores/replays",
                ),
                (
                    "campaign_state_directory",
                    "/home/robinhood/.local/share/robin-highscores/campaign-states",
                ),
                (
                    "cursor_secret_path",
                    "/home/robinhood/.local/share/robin-highscores/api-secrets/cursor-hmac.key",
                ),
                (
                    "competition_run_grant_secret_path",
                    "/home/robinhood/.local/share/robin-highscores/api-secrets/competition-run-grant.key",
                ),
                (
                    "run_preflight_grant_secret_path",
                    "/home/robinhood/.local/share/robin-highscores/api-secrets/run-preflight-grant.key",
                ),
                (
                    "moderation_bearer_token_path",
                    "/home/robinhood/.local/share/robin-highscores/api-secrets/moderation-bearer.token",
                ),
                ("backup_manifest_path", BACKUP_STATUS_PATH),
            ] {
                ensure!(
                    value.get(field).and_then(toml::Value::as_str) == Some(expected),
                    "server config {field} is not the final private path"
                );
            }
            let expected_release_manifest =
                format!("{INSTALL_ROOT}/releases/{source_commit}/{RELEASE_MANIFEST_FILE}");
            ensure!(
                value
                    .get("release_manifest_path")
                    .and_then(toml::Value::as_str)
                    == Some(expected_release_manifest.as_str()),
                "server config must bind the exact installed VPS release manifest"
            );
            ensure!(
                value
                    .get("allowed_origins")
                    .and_then(toml::Value::as_array)
                    .is_some_and(Vec::is_empty),
                "production server config must not enable CORS origins"
            );
            ensure!(
                value
                    .get("maximum_backup_age_hours")
                    .and_then(toml::Value::as_integer)
                    == Some(32),
                "server config maximum backup age must cover daily schedule, jitter, timeout, and margin"
            );
            ensure!(
                toml_integer_at_least(&value, "minimum_storage_free_bytes", 1 << 30),
                "server config must reserve at least 1 GiB of storage"
            );
            for field in [
                "max_replay_bytes",
                "max_campaign_bytes",
                "max_metadata_bytes",
                "max_concurrent_uploads",
            ] {
                ensure!(
                    positive_toml_integer(&value, field),
                    "server config {field} must be positive"
                );
            }
            let expected_manifest_root =
                format!("{INSTALL_ROOT}/releases/{source_commit}/config/manifests");
            ensure!(
                value
                    .get("manifest_directory")
                    .and_then(toml::Value::as_str)
                    == Some(expected_manifest_root.as_str()),
                "server config must use the installed immutable manifest root"
            );
            let profiles = value
                .get("admission_profiles")
                .and_then(toml::Value::as_array)
                .filter(|profiles| !profiles.is_empty())
                .context("server config has no final admission profiles")?;
            let expected_campaign_root =
                format!("{INSTALL_ROOT}/releases/{source_commit}/private/campaign-states");
            let referenced_campaigns = profiles
                .iter()
                .map(|profile| {
                    let path = profile
                        .get("canonical_campaign_state_path")
                        .and_then(toml::Value::as_str)
                        .context("admission profile omits canonical campaign state path")?;
                    let path = Path::new(path);
                    ensure!(
                        path.parent() == Some(Path::new(&expected_campaign_root)),
                        "admission profile campaign template escapes the immutable release"
                    );
                    let digest = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .context("admission profile campaign template has no digest filename")?
                        .parse::<Digest32>()?;
                    Ok(digest)
                })
                .collect::<Result<BTreeSet<_>>>()?;
            ensure!(
                referenced_campaigns == *campaign_states,
                "server profiles do not bind the exact publication campaign templates"
            );
        }
        VpsConfigRoleV2::Worker => {
            let value: toml::Value = toml::from_str(std::str::from_utf8(&bytes)?)?;
            reject_obsolete_toml(&value)?;
            let release_root = format!("{INSTALL_ROOT}/releases/{source_commit}");
            let expected_server_config = format!("{release_root}/config/highscores-server.toml");
            ensure!(
                value.get("server_config").and_then(toml::Value::as_str)
                    == Some(expected_server_config.as_str()),
                "worker config does not use the exact commit-named server config"
            );
            ensure!(
                value
                    .get("campaign_state_directory")
                    .and_then(toml::Value::as_str)
                    == Some("/home/robinhood/.local/share/robin-highscores/campaign-states"),
                "worker config does not use the persistent campaign store"
            );
            let expected_catalog_path = format!(
                "{release_root}/private/verifier/operator-config/{}",
                job_catalog.sha256,
            );
            let expected_catalog_sha256 = job_catalog.sha256.to_string();
            ensure!(
                value
                    .get("verifier_job_config_catalog")
                    .and_then(toml::Value::as_str)
                    == Some(expected_catalog_path.as_str())
                    && value
                        .get("verifier_job_config_catalog_sha256")
                        .and_then(toml::Value::as_str)
                        == Some(expected_catalog_sha256.as_str()),
                "worker config does not bind the publication job catalog"
            );
            let launcher = value
                .get("verifier_launcher")
                .and_then(toml::Value::as_table)
                .context("worker config omits [verifier_launcher]")?;
            let expected_verifier_path = format!("{release_root}/bin/robin-replay-verifier");
            let expected_verifier_sha256 = verifier_sha256.to_string();
            ensure!(
                launcher.get("bwrap_program").and_then(toml::Value::as_str)
                    == Some("/usr/bin/bwrap")
                    && launcher
                        .get("prlimit_program")
                        .and_then(toml::Value::as_str)
                        == Some("/usr/bin/prlimit")
                    && launcher
                        .get("verifier_program")
                        .and_then(toml::Value::as_str)
                        == Some(expected_verifier_path.as_str()),
                "worker verifier launcher does not use the fixed host tools and exact release verifier"
            );
            ensure!(
                nonzero_lower_hex_table(launcher, "bwrap_sha256")
                    && nonzero_lower_hex_table(launcher, "prlimit_sha256")
                    && launcher
                        .get("verifier_sha256")
                        .and_then(toml::Value::as_str)
                        == Some(expected_verifier_sha256.as_str()),
                "worker verifier launcher has an absent, zero, or substituted digest"
            );
            for (field, expected) in [
                ("wall_timeout_seconds", 120),
                ("cpu_limit_seconds", 120),
                ("address_space_limit_bytes", 1_073_741_824),
                ("process_limit", 32),
                ("open_files_limit", 128),
                ("file_size_limit_bytes", 134_217_728),
                ("max_request_bytes", 1_048_576),
            ] {
                ensure!(
                    launcher.get(field).and_then(toml::Value::as_integer) == Some(expected),
                    "worker verifier launcher {field} differs from the canonical resource envelope"
                );
            }
            let limits = value
                .get("limits")
                .and_then(toml::Value::as_table)
                .context("worker config omits [limits]")?;
            let max_campaign_bytes = limits
                .get("max_campaign_bytes")
                .and_then(toml::Value::as_integer)
                .context("worker limits omit max_campaign_bytes")?;
            ensure!(
                max_campaign_bytes > 0 && max_campaign_bytes <= 134_217_728,
                "worker campaign limit exceeds the direct launch file-size limit"
            );
            let _ = worker_source_manifest_selections(&value, source_commit)?;
            for forbidden in [
                "broker_socket",
                "broker_response_timeout_seconds",
                "verifier_sha256",
                "sandbox_launcher",
                "sandbox_launcher_sha256",
            ] {
                ensure!(
                    value.get(forbidden).is_none(),
                    "worker config retains obsolete root-level field {forbidden}"
                );
            }
        }
    }
    Ok(())
}

pub(super) fn reject_obsolete_toml(value: &toml::Value) -> Result<()> {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                let key = key.to_ascii_lowercase();
                ensure!(
                    !key.contains("broker")
                        && !key.contains("polkit")
                        && !matches!(
                            key.as_str(),
                            "worker_uid"
                                | "sandbox_launcher"
                                | "sandbox_launcher_sha256"
                                | "runtime_max_seconds"
                                | "memory_max_bytes"
                                | "tasks_max"
                                | "nofile_max"
                                | "file_size_max_bytes"
                        ),
                    "release config retains obsolete privileged field {key}"
                );
                reject_obsolete_toml(value)?;
            }
        }
        toml::Value::Array(values) => {
            for value in values {
                reject_obsolete_toml(value)?;
            }
        }
        toml::Value::String(value) => {
            ensure!(
                !value.contains("verifier-broker")
                    && !value.contains("polkit")
                    && value != "/usr/bin/systemd-run"
                    && !value.starts_with("/opt/robin-highscores")
                    && !value.starts_with("/var/lib/robin-highscores")
                    && !value.starts_with("/srv/robin-highscores")
                    && !value.starts_with("/etc/robin-highscores"),
                "release config retains an obsolete privileged path or authority"
            );
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn positive_toml_integer(value: &toml::Value, field: &str) -> bool {
    value
        .get(field)
        .and_then(toml::Value::as_integer)
        .is_some_and(|value| value > 0)
}

pub(super) fn toml_integer_at_least(value: &toml::Value, field: &str, minimum: i64) -> bool {
    value
        .get(field)
        .and_then(toml::Value::as_integer)
        .is_some_and(|value| value >= minimum)
}

pub(super) fn worker_source_manifest_selections(
    value: &toml::Value,
    source_commit: &str,
) -> Result<[(OfficialContentEditionV1, Digest32); 2]> {
    let expected_root =
        format!("{INSTALL_ROOT}/releases/{source_commit}/private/source-tree-manifests-v2");
    let mut selections = Vec::new();
    for (field, edition) in [
        ("demo_raw_content_manifest", OfficialContentEditionV1::Demo),
        ("full_raw_content_manifest", OfficialContentEditionV1::Full),
    ] {
        let configured = value
            .get(field)
            .and_then(toml::Value::as_str)
            .with_context(|| format!("worker config omits {field}"))?;
        let path = Path::new(configured);
        ensure!(
            normalized_absolute(path) && path.parent() == Some(Path::new(&expected_root)),
            "worker config {field} escapes the immutable release manifest root"
        );
        let digest = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".json"))
            .context("worker raw-content manifest is not a digest-named JSON file")?
            .parse::<Digest32>()?;
        ensure!(
            !digest.is_zero(),
            "worker raw-content manifest uses a zero digest"
        );
        selections.push((edition, digest));
    }
    ensure!(
        selections[0].1 != selections[1].1,
        "Demo and Full raw roots cannot share one source manifest"
    );
    Ok([selections[0], selections[1]])
}

pub(super) fn validate_worker_raw_authority(
    worker_config: &Path,
    source_commit: &str,
    local_manifest_root: &Path,
    raw_roots: &[PrivateRawRootV2],
) -> Result<()> {
    let bytes = read_regular_file_bounded(worker_config, MAX_CONFIG_BYTES)?;
    let value: toml::Value = toml::from_str(std::str::from_utf8(&bytes)?)?;
    let selections = worker_source_manifest_selections(&value, source_commit)?;
    for (edition, digest) in selections {
        let path = local_manifest_root.join(format!("{digest}.json"));
        let manifest: OfficialSourceTreeManifestV2 = load_canonical(&path)?;
        ensure!(
            manifest.canonical_digest()? == digest && manifest.edition == edition,
            "selected raw-content source manifest has the wrong digest or edition"
        );
        let raw_root = raw_roots
            .iter()
            .find(|root| root.edition == edition)
            .context("raw-content declaration omits selected edition")?;
        validate_raw_content_against_manifest(&raw_root.root, &manifest)
            .with_context(|| format!("validate {edition:?} raw content against {digest}"))?;
    }
    Ok(())
}

pub(super) fn validate_raw_content_against_manifest(
    raw_root: &Path,
    manifest: &OfficialSourceTreeManifestV2,
) -> Result<()> {
    for expected in &manifest.files {
        ensure!(
            valid_relative_manifest_path(&expected.path),
            "source manifest contains an unsafe raw path"
        );
        let path = raw_root.join(&expected.path);
        let actual = artifact_from_file(&path, "application/octet-stream")?;
        ensure!(
            actual.sha256 == expected.sha256 && actual.byte_length == expected.byte_length,
            "raw content differs from selected source manifest at {}",
            expected.path
        );
    }
    Ok(())
}

pub(super) fn nonzero_lower_hex_table(
    value: &toml::map::Map<String, toml::Value>,
    field: &str,
) -> bool {
    value
        .get(field)
        .and_then(toml::Value::as_str)
        .is_some_and(|value| {
            value.len() == 64
                && value.bytes().any(|byte| byte != b'0')
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
}

pub(super) fn validate_final_host_file(
    role: VpsHostFileRoleV2,
    path: &Path,
    source_commit: &str,
) -> Result<()> {
    // Bind policy evaluation to the same bounded byte snapshot, not a second
    // read of a path that may have changed after UTF-8/placeholder validation.
    let bytes = read_regular_file_bounded(path, MAX_CONFIG_BYTES)?;
    validate_host_template_bytes(role, &bytes, source_commit)
}

pub(super) fn validate_backup_sandbox_contract(
    server_config: &Path,
    api_unit: &Path,
    worker_unit: &Path,
    backup_unit: &Path,
    backup_timer: &Path,
    source_commit: &str,
) -> Result<()> {
    let config_bytes = read_regular_file_bounded(server_config, MAX_CONFIG_BYTES)?;
    let config: toml::Value = toml::from_str(std::str::from_utf8(&config_bytes)?)?;
    ensure!(
        config
            .get("maximum_backup_age_hours")
            .and_then(toml::Value::as_integer)
            == Some(32),
        "backup readiness maximum age must be exactly 32 hours"
    );
    let status_path = config
        .get("backup_manifest_path")
        .and_then(toml::Value::as_str)
        .context("server config omits backup_manifest_path")?;
    ensure!(
        status_path == BACKUP_STATUS_PATH,
        "server backup_manifest_path must name the isolated status authority"
    );
    let status_parent = Path::new(status_path)
        .parent()
        .context("server backup_manifest_path has no parent")?;
    ensure!(
        status_parent == Path::new(BACKUP_STATUS_ROOT)
            && !Path::new(status_path).starts_with(BACKUP_ROOT),
        "server backup_manifest_path must remain outside the backup payload root"
    );
    let release_root = format!("{INSTALL_ROOT}/releases/{source_commit}");
    let release_manifest = format!("{release_root}/{RELEASE_MANIFEST_FILE}");
    ensure!(
        config
            .get("release_manifest_path")
            .and_then(toml::Value::as_str)
            == Some(release_manifest.as_str()),
        "server release_manifest_path must name the exact installed release manifest"
    );

    let api = unit_text(api_unit)?;
    ensure!(
        has_exact_unit_path(&api, "ReadOnlyPaths", BACKUP_STATUS_ROOT),
        "API unit must receive read-only access to the exact backup status directory"
    );
    ensure!(
        has_exact_unit_path(&api, "InaccessiblePaths", BACKUP_ROOT),
        "API unit must keep backup payloads inaccessible"
    );
    ensure!(
        !has_exact_unit_path(&api, "ReadWritePaths", BACKUP_STATUS_ROOT)
            && !unit_grants_path(&api, BACKUP_ROOT),
        "API unit grants forbidden write/status or backup-payload access"
    );

    let backup = unit_text(backup_unit)?;
    for writable in [BACKUP_ROOT, BACKUP_STATUS_ROOT] {
        ensure!(
            has_exact_unit_path(&backup, "ReadWritePaths", writable),
            "backup unit must write the exact payload and status directories"
        );
    }
    ensure!(
        backup.lines().any(|line| {
            line.starts_with("ExecStart=")
                && line.contains(&format!(" --release-manifest-path {release_manifest} "))
                && line.contains(&format!(" --backup-root {BACKUP_ROOT} "))
                && line.contains(&format!(" --status-path {BACKUP_STATUS_PATH} "))
        }),
        "backup unit command does not publish to the server's exact status authority"
    );
    let exec = backup
        .lines()
        .find(|line| line.starts_with("ExecStart="))
        .context("backup unit omits ExecStart")?;
    ensure!(
        !exec.contains("--configuration-root")
            && !exec.contains(&format!("--restore-source-map {release_root}="))
            && !exec.contains(&format!(
                "--restore-source-map {DEPLOYMENT_HOME}/.config/systemd/user="
            )),
        "backup unit must not copy immutable release/config trees or the symlink-bearing user-unit tree"
    );
    let tokens = exec.split_ascii_whitespace().collect::<Vec<_>>();
    let actual_maps = tokens
        .windows(2)
        .filter(|pair| pair[0] == "--restore-source-map")
        .map(|pair| pair[1].to_owned())
        .collect::<BTreeSet<_>>();
    let secret_root = format!("{STATE_ROOT}/api-secrets");
    let user_unit_root = format!("{DEPLOYMENT_HOME}/.config/systemd/user");
    let expected_maps = [
        format!("{secret_root}/cursor-hmac.key={secret_root}/cursor-hmac.key"),
        format!("{secret_root}/competition-run-grant.key={secret_root}/competition-run-grant.key"),
        format!("{secret_root}/run-preflight-grant.key={secret_root}/run-preflight-grant.key"),
        format!("{secret_root}/moderation-bearer.token={secret_root}/moderation-bearer.token"),
        format!("{user_unit_root}/robin-highscores.target={user_unit_root}/robin-highscores.target"),
        format!("{user_unit_root}/robin-highscores-api.service={user_unit_root}/robin-highscores-api.service"),
        format!("{user_unit_root}/robin-highscores-worker.service={user_unit_root}/robin-highscores-worker.service"),
        format!("{user_unit_root}/robin-highscores-backup.service={user_unit_root}/robin-highscores-backup.service"),
        format!("{user_unit_root}/robin-highscores-backup.timer={user_unit_root}/robin-highscores-backup.timer"),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    ensure!(
        actual_maps == expected_maps,
        "backup unit restore maps must name exactly four secrets and five regular user units"
    );
    ensure!(
        !has_exact_unit_path(&backup, "ReadOnlyPaths", &user_unit_root),
        "backup unit must not expose the whole symlink-bearing user-unit tree"
    );
    for unit in [
        "robin-highscores.target",
        "robin-highscores-api.service",
        "robin-highscores-worker.service",
        "robin-highscores-backup.service",
        "robin-highscores-backup.timer",
    ] {
        ensure!(
            has_exact_unit_path(
                &backup,
                "ReadOnlyPaths",
                &format!("{user_unit_root}/{unit}"),
            ),
            "backup sandbox omits exact user-unit source {unit}"
        );
    }
    ensure!(
        backup
            .lines()
            .any(|line| line.trim() == "TimeoutStartSec=6h"),
        "backup timeout differs from the readiness timing contract"
    );
    let timer = unit_text(backup_timer)?;
    ensure!(
        timer
            .lines()
            .any(|line| line.trim() == "OnCalendar=*-*-* 02:15:00")
            && timer
                .lines()
                .any(|line| line.trim() == "RandomizedDelaySec=45m")
            && 32 * 60 > 24 * 60 + 45 + 6 * 60 + 60,
        "backup age does not cover daily interval, jitter, timeout, and one-hour margin"
    );

    let worker = unit_text(worker_unit)?;
    for inaccessible in [BACKUP_ROOT, BACKUP_STATUS_ROOT] {
        ensure!(
            has_exact_unit_path(&worker, "InaccessiblePaths", inaccessible),
            "worker unit must not gain access to backup payload or status authority"
        );
        ensure!(
            !unit_grants_path(&worker, inaccessible),
            "worker unit grants forbidden backup/status access"
        );
    }
    Ok(())
}

pub(super) fn unit_text(path: &Path) -> Result<String> {
    let bytes = read_regular_file_bounded(path, MAX_CONFIG_BYTES)?;
    Ok(std::str::from_utf8(&bytes)?.to_owned())
}

pub(super) fn has_exact_unit_path(text: &str, directive: &str, path: &str) -> bool {
    let expected = format!("{directive}={path}");
    text.lines().any(|line| line.trim() == expected)
}

pub(super) fn unit_grants_path(text: &str, path: &str) -> bool {
    text.lines().any(|line| {
        ["ReadOnlyPaths", "ReadWritePaths"].iter().any(|directive| {
            line.trim()
                .strip_prefix(&format!("{directive}="))
                .is_some_and(|paths| paths.split_ascii_whitespace().any(|entry| entry == path))
        })
    })
}

pub(super) fn reject_placeholders(bytes: &[u8], label: &str) -> Result<()> {
    let text = std::str::from_utf8(bytes)?;
    let lowercase = text.to_ascii_lowercase();
    ensure!(
        !lowercase.contains(&"0".repeat(64))
            && !lowercase.contains("changeme")
            && !lowercase.contains("placeholder")
            && !lowercase.contains("example.invalid"),
        "{label} contains a zero/example/placeholder value"
    );
    Ok(())
}
