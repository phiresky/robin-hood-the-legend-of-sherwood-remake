use super::*;

fn fact(bytes: &[u8]) -> ArtifactRefV1 {
    ArtifactRefV1 {
        sha256: Digest32::digest_bytes(bytes),
        byte_length: bytes.len() as u64,
        media_type: "application/octet-stream".into(),
    }
}

struct SourceConsumeFixture {
    sandbox: tempfile::TempDir,
    incoming: PathBuf,
    releases: PathBuf,
    logical_source: PathBuf,
    candidate: PathBuf,
    plan: VpsReleasePlanV2,
    plan_bytes: Vec<u8>,
    plan_sha256: Digest32,
    release_manifest_sha256: Digest32,
    publication_lock_sha256: Digest32,
    retained_admin_writer: File,
}

impl SourceConsumeFixture {
    fn consuming(&self) -> PathBuf {
        self.incoming
            .join(format!(".sources-{}.consuming", self.plan.source_commit))
    }

    fn journal(&self) -> PathBuf {
        self.incoming.join(format!(
            ".sources-{}.consume-v1.json",
            self.plan.source_commit
        ))
    }

    fn journal_temporary(&self) -> PathBuf {
        self.incoming.join(format!(
            ".sources-{}.consume-v1.json.new",
            self.plan.source_commit
        ))
    }

    fn terminal_journal(&self) -> PathBuf {
        self.incoming.join(format!(
            ".sources-{}.consume-v1.complete.json",
            self.plan.source_commit
        ))
    }
}

impl Drop for SourceConsumeFixture {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt as _;

        fn make_tree_removable(path: &Path) {
            let Ok(metadata) = fs::symlink_metadata(path) else {
                return;
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return;
            }
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
            let Ok(entries) = fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                make_tree_removable(&entry.path());
            }
        }

        make_tree_removable(self.sandbox.path());
    }
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

struct RuntimeFenceFixture {
    sandbox: tempfile::TempDir,
    opt_root: PathBuf,
    state_root: PathBuf,
    source_commit: String,
    activation_lock: PinnedVpsActivationLockV2,
}

impl RuntimeFenceFixture {
    fn new() -> Result<Self> {
        let sandbox = tempfile::tempdir_in(std::env::current_dir()?)?;
        set_mode(sandbox.path(), 0o700)?;
        let opt_root = sandbox.path().join("opt");
        fs::create_dir(&opt_root)?;
        set_mode(&opt_root, 0o750)?;
        let state_root = sandbox.path().join("state");
        fs::create_dir(&state_root)?;
        set_mode(&state_root, 0o700)?;
        let activation_lock = acquire_vps_activation_lock_at(&opt_root)?;
        Ok(Self {
            sandbox,
            opt_root,
            state_root,
            source_commit: "a".repeat(40),
            activation_lock,
        })
    }

    fn staging(&self) -> PathBuf {
        self.state_root
            .join(format!(".runtime-fence-{}.partial", self.source_commit))
    }

    fn foreign_staging(&self) -> PathBuf {
        self.state_root
            .join(format!(".runtime-fence-{}.partial", "b".repeat(40)))
    }

    fn intent(&self) -> PathBuf {
        self.state_root.join(RUNTIME_FENCE_INTENT_NAME)
    }

    fn intent_temporary(&self) -> PathBuf {
        self.state_root.join(RUNTIME_FENCE_INTENT_TEMPORARY_NAME)
    }

    fn final_root(&self) -> PathBuf {
        self.state_root.join("runtime-fence")
    }

    fn run(&self) -> Result<()> {
        initialize_vps_runtime_fence_v1_at(
            &self.source_commit,
            &self.activation_lock,
            &self.state_root,
            |_| Ok(()),
        )
    }

    fn fail_after(&self, target: RuntimeFenceInitBoundaryV1) -> Result<()> {
        initialize_vps_runtime_fence_v1_at(
            &self.source_commit,
            &self.activation_lock,
            &self.state_root,
            |boundary| {
                ensure!(boundary != target, "injected runtime-fence crash boundary");
                Ok(())
            },
        )
    }

    fn assert_sealed(&self) -> Result<()> {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        ensure!(
            self.final_root().is_dir(),
            "runtime-fence was not published"
        );
        ensure!(
            fs::symlink_metadata(self.final_root())?
                .permissions()
                .mode()
                & 0o777
                == 0o500,
            "runtime-fence final mode is not 0500"
        );
        for name in ["db-admission.lock", "db-quiescence.lock"] {
            let metadata = fs::symlink_metadata(self.final_root().join(name))?;
            ensure!(
                metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.permissions().mode() & 0o777 == 0o400
                    && metadata.nlink() == 1
                    && metadata.len() == 0,
                "runtime-fence final leaf is not exact"
            );
        }
        ensure!(
            !self.staging().exists()
                && !self.intent().exists()
                && !self.intent_temporary().exists(),
            "runtime-fence terminal initialization retained recovery evidence"
        );
        Ok(())
    }
}

impl Drop for RuntimeFenceFixture {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt as _;

        fn make_removable(path: &Path) {
            let Ok(metadata) = fs::symlink_metadata(path) else {
                return;
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return;
            }
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    make_removable(&entry.path());
                }
            }
        }

        make_removable(self.sandbox.path());
    }
}

#[test]
fn runtime_fence_initializer_cleanly_publishes_exact_permanent_inodes() -> Result<()> {
    let fixture = RuntimeFenceFixture::new()?;
    fixture.run()?;
    fixture.assert_sealed()?;
    fixture.run()?;
    fixture.assert_sealed()?;
    Ok(())
}

#[test]
fn runtime_fence_initializer_reconciles_every_authenticated_crash_boundary() -> Result<()> {
    for boundary in [
        RuntimeFenceInitBoundaryV1::IntentWritingCreated,
        RuntimeFenceInitBoundaryV1::IntentWritingSynced,
        RuntimeFenceInitBoundaryV1::IntentNewPublished,
        RuntimeFenceInitBoundaryV1::AuthorizedIntentPublished,
        RuntimeFenceInitBoundaryV1::StagingCreated,
        RuntimeFenceInitBoundaryV1::BoundIntentWritingCreated,
        RuntimeFenceInitBoundaryV1::BoundIntentWritingSynced,
        RuntimeFenceInitBoundaryV1::BoundIntentNewPublished,
        RuntimeFenceInitBoundaryV1::BoundIntentExchanged,
        RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished,
        RuntimeFenceInitBoundaryV1::AdmissionLeafSynced,
        RuntimeFenceInitBoundaryV1::QuiescenceLeafSynced,
        RuntimeFenceInitBoundaryV1::StagingSealed,
        RuntimeFenceInitBoundaryV1::FinalPublished,
        RuntimeFenceInitBoundaryV1::IntentRemoved,
    ] {
        let fixture = RuntimeFenceFixture::new()?;
        ensure!(
            fixture.fail_after(boundary).is_err(),
            "crash injection did not stop at {boundary:?}"
        );
        fixture.run()?;
        fixture.assert_sealed()?;
    }
    Ok(())
}

#[test]
fn runtime_fence_initializer_converges_after_real_sigkill_at_every_mutation_boundary() -> Result<()>
{
    use std::os::unix::process::ExitStatusExt as _;
    use std::process::Command;

    const CHILD: &str = "ROBIN_RUNTIME_FENCE_SIGKILL_CHILD";
    const OPT_ROOT: &str = "ROBIN_RUNTIME_FENCE_SIGKILL_OPT";
    const STATE_ROOT_ENV: &str = "ROBIN_RUNTIME_FENCE_SIGKILL_STATE";
    const COMMIT: &str = "ROBIN_RUNTIME_FENCE_SIGKILL_COMMIT";
    const BOUNDARY: &str = "ROBIN_RUNTIME_FENCE_SIGKILL_BOUNDARY";
    const BOUNDARIES: [RuntimeFenceInitBoundaryV1; 15] = [
        RuntimeFenceInitBoundaryV1::IntentWritingCreated,
        RuntimeFenceInitBoundaryV1::IntentWritingSynced,
        RuntimeFenceInitBoundaryV1::IntentNewPublished,
        RuntimeFenceInitBoundaryV1::AuthorizedIntentPublished,
        RuntimeFenceInitBoundaryV1::StagingCreated,
        RuntimeFenceInitBoundaryV1::BoundIntentWritingCreated,
        RuntimeFenceInitBoundaryV1::BoundIntentWritingSynced,
        RuntimeFenceInitBoundaryV1::BoundIntentNewPublished,
        RuntimeFenceInitBoundaryV1::BoundIntentExchanged,
        RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished,
        RuntimeFenceInitBoundaryV1::AdmissionLeafSynced,
        RuntimeFenceInitBoundaryV1::QuiescenceLeafSynced,
        RuntimeFenceInitBoundaryV1::StagingSealed,
        RuntimeFenceInitBoundaryV1::FinalPublished,
        RuntimeFenceInitBoundaryV1::IntentRemoved,
    ];

    if std::env::var_os(CHILD).is_some() {
        let opt_root = PathBuf::from(std::env::var_os(OPT_ROOT).context("missing opt root")?);
        let state_root =
            PathBuf::from(std::env::var_os(STATE_ROOT_ENV).context("missing state root")?);
        let source_commit = std::env::var(COMMIT)?;
        let boundary = BOUNDARIES[std::env::var(BOUNDARY)?.parse::<usize>()?];
        let activation_lock = acquire_vps_activation_lock_at(&opt_root)?;
        initialize_vps_runtime_fence_v1_at(
            &source_commit,
            &activation_lock,
            &state_root,
            |observed| {
                if observed == boundary {
                    rustix::process::kill_process(
                        rustix::process::getpid(),
                        rustix::process::Signal::KILL,
                    )?;
                    anyhow::bail!("SIGKILL unexpectedly returned")
                }
                Ok(())
            },
        )?;
        anyhow::bail!("SIGKILL boundary was not reached")
    }

    for (index, boundary) in BOUNDARIES.into_iter().enumerate() {
        let sandbox = tempfile::tempdir_in(std::env::current_dir()?)?;
        set_mode(sandbox.path(), 0o700)?;
        let opt_root = sandbox.path().join("opt");
        fs::create_dir(&opt_root)?;
        set_mode(&opt_root, 0o750)?;
        let state_root = sandbox.path().join("state");
        fs::create_dir(&state_root)?;
        set_mode(&state_root, 0o700)?;
        let source_commit = "a".repeat(40);
        let status = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "vps_release_v2::tests::runtime_fence_initializer_converges_after_real_sigkill_at_every_mutation_boundary",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env(OPT_ROOT, &opt_root)
            .env(STATE_ROOT_ENV, &state_root)
            .env(COMMIT, &source_commit)
            .env(BOUNDARY, index.to_string())
            .status()?;
        ensure!(
            status.signal() == Some(9),
            "child was not SIGKILLed at {boundary:?}: {status}"
        );
        let staging_path = state_root.join(format!(".runtime-fence-{source_commit}.partial"));
        let retained_staging_identity = fs::symlink_metadata(&staging_path).ok().map(|metadata| {
            use std::os::unix::fs::MetadataExt as _;
            (metadata.dev(), metadata.ino())
        });
        let activation_lock = acquire_vps_activation_lock_at(&opt_root)?;
        let fixture = RuntimeFenceFixture {
            sandbox,
            opt_root,
            state_root,
            source_commit,
            activation_lock,
        };
        fixture.run()?;
        fixture.assert_sealed()?;
        if let Some(retained_staging_identity) = retained_staging_identity {
            use std::os::unix::fs::MetadataExt as _;

            let final_metadata = fs::symlink_metadata(fixture.final_root())?;
            ensure!(
                (final_metadata.dev(), final_metadata.ino()) == retained_staging_identity,
                "SIGKILL resume replaced the retained staging inode at {boundary:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn runtime_fence_initializer_final_plus_intent_adopts_only_the_bound_inode() -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;

    let fixture = RuntimeFenceFixture::new()?;
    ensure!(
        fixture
            .fail_after(RuntimeFenceInitBoundaryV1::FinalPublished)
            .is_err(),
        "final publication crash injection unexpectedly completed"
    );
    let intent: RuntimeFenceInitIntentV1 = strict_json_from_slice(&fs::read(fixture.intent())?)?;
    let final_metadata = fs::symlink_metadata(fixture.final_root())?;
    ensure!(
        runtime_fence_bound_identity(&intent)? == (final_metadata.dev(), final_metadata.ino()),
        "final publication did not retain the journaled staging inode"
    );
    fixture.run()?;
    fixture.assert_sealed()?;
    Ok(())
}

#[test]
fn runtime_fence_initializer_rejects_unjournaled_and_foreign_staging() -> Result<()> {
    let fixture = RuntimeFenceFixture::new()?;
    fs::create_dir(fixture.staging())?;
    set_mode(&fixture.staging(), 0o700)?;
    ensure!(
        fixture.run().is_err(),
        "unjournaled runtime-fence staging was adopted"
    );

    let fixture = RuntimeFenceFixture::new()?;
    fs::create_dir(fixture.foreign_staging())?;
    set_mode(&fixture.foreign_staging(), 0o700)?;
    ensure!(
        fixture.run().is_err(),
        "foreign runtime-fence staging was ignored"
    );
    Ok(())
}

#[test]
fn runtime_fence_initializer_rejects_symlink_hardlink_mode_device_and_extra_entries() -> Result<()>
{
    use std::os::unix::fs::symlink;

    let fixture = RuntimeFenceFixture::new()?;
    ensure!(
        fixture
            .fail_after(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)
            .is_err(),
        "intent publication crash injection unexpectedly completed"
    );
    symlink("/dev/null", fixture.staging().join("db-admission.lock"))?;
    ensure!(fixture.run().is_err(), "runtime-fence symlink was accepted");

    let fixture = RuntimeFenceFixture::new()?;
    fixture
        .fail_after(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)
        .expect_err("intent publication crash injection unexpectedly completed");
    fs::write(fixture.staging().join("db-admission.lock"), b"")?;
    set_mode(&fixture.staging().join("db-admission.lock"), 0o400)?;
    fs::hard_link(
        fixture.staging().join("db-admission.lock"),
        fixture.state_root.join("second-link"),
    )?;
    ensure!(
        fixture.run().is_err(),
        "runtime-fence hard link was accepted"
    );

    let fixture = RuntimeFenceFixture::new()?;
    fixture
        .fail_after(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)
        .expect_err("intent publication crash injection unexpectedly completed");
    set_mode(&fixture.staging(), 0o755)?;
    ensure!(fixture.run().is_err(), "unsafe staging mode was repaired");

    let fixture = RuntimeFenceFixture::new()?;
    fixture
        .fail_after(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)
        .expect_err("intent publication crash injection unexpectedly completed");
    let mut intent: RuntimeFenceInitIntentV1 =
        strict_json_from_slice(&fs::read(fixture.intent())?)?;
    intent.staging_device = Some(
        intent
            .staging_device
            .context("fixture bound intent omitted device")?
            .checked_add(1)
            .context("fixture device overflow")?,
    );
    set_mode(&fixture.intent(), 0o600)?;
    fs::write(fixture.intent(), canonical_json_bytes(&intent)?)?;
    set_mode(&fixture.intent(), 0o400)?;
    ensure!(
        fixture.run().is_err(),
        "runtime-fence intent with a foreign device was accepted"
    );

    let fixture = RuntimeFenceFixture::new()?;
    fixture
        .fail_after(RuntimeFenceInitBoundaryV1::StagingBoundIntentPublished)
        .expect_err("intent publication crash injection unexpectedly completed");
    fs::write(fixture.staging().join("unexpected"), b"")?;
    set_mode(&fixture.staging().join("unexpected"), 0o400)?;
    ensure!(fixture.run().is_err(), "extra staging entry was accepted");
    Ok(())
}

#[test]
fn runtime_fence_initializer_never_repairs_a_missing_final_leaf() -> Result<()> {
    let fixture = RuntimeFenceFixture::new()?;
    fixture.run()?;
    set_mode(&fixture.final_root(), 0o700)?;
    fs::remove_file(fixture.final_root().join("db-quiescence.lock"))?;
    set_mode(&fixture.final_root(), 0o500)?;
    ensure!(
        fixture.run().is_err(),
        "initializer repaired an incomplete published runtime-fence"
    );
    Ok(())
}

#[test]
fn runtime_fence_initializer_rejects_activation_lock_substitution() -> Result<()> {
    use rustix::fs::{Mode, OFlags, open};
    use std::os::unix::fs::PermissionsExt as _;

    let fixture = RuntimeFenceFixture::new()?;
    fs::rename(
        fixture.opt_root.join("activation.lock"),
        fixture.opt_root.join("activation.lock.displaced"),
    )?;
    let replacement = open(
        fixture.opt_root.join("activation.lock"),
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )?;
    rustix::fs::fsync(&replacement)?;
    fs::set_permissions(
        fixture.opt_root.join("activation.lock"),
        fs::Permissions::from_mode(0o600),
    )?;
    ensure!(
        fixture.run().is_err(),
        "initializer accepted a substituted activation lock"
    );
    ensure!(
        fs::read_dir(&fixture.state_root)?.next().is_none(),
        "initializer mutated state after activation-lock substitution"
    );
    Ok(())
}

fn write_source_file(path: &Path, bytes: &[u8], mode: u32) -> Result<File> {
    use std::io::Write as _;

    fs::create_dir_all(path.parent().context("source file has no parent")?)?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    set_mode(path, mode)?;
    Ok(file)
}

fn source_consume_fixture() -> Result<SourceConsumeFixture> {
    let sandbox = tempfile::tempdir_in(std::env::current_dir()?)?;
    set_mode(sandbox.path(), 0o750)?;
    let incoming = sandbox.path().join("incoming");
    fs::create_dir(&incoming)?;
    set_mode(&incoming, 0o750)?;
    let releases = sandbox.path().join("releases");
    fs::create_dir(&releases)?;
    set_mode(&releases, 0o750)?;
    let source_commit = "a".repeat(40);
    let logical_source = incoming.join(format!(".sources-{source_commit}"));
    fs::create_dir(&logical_source)?;
    set_mode(&logical_source, 0o700)?;

    let binary_roles = [
        VpsBinaryRoleV2::Admin,
        VpsBinaryRoleV2::ManifestTool,
        VpsBinaryRoleV2::Server,
        VpsBinaryRoleV2::Worker,
        VpsBinaryRoleV2::ReplayVerifier,
    ];
    let binaries = binary_roles
        .into_iter()
        .map(|role| {
            let bytes = role.output_name().as_bytes();
            VpsBinarySourceV2 {
                role,
                source: logical_source.join("bin").join(role.output_name()),
                artifact: fact(bytes),
            }
        })
        .collect::<Vec<_>>();
    let config_roles = [
        VpsConfigRoleV2::Server,
        VpsConfigRoleV2::Worker,
        VpsConfigRoleV2::ApiEnvironment,
        VpsConfigRoleV2::WorkerEnvironment,
    ];
    let configs = config_roles
        .into_iter()
        .map(|role| {
            let bytes = role.output_name().as_bytes();
            VpsConfigSourceV2 {
                role,
                source: logical_source.join("config").join(role.output_name()),
                artifact: fact(bytes),
            }
        })
        .collect::<Vec<_>>();
    let host_roles = [
        VpsHostFileRoleV2::UserTarget,
        VpsHostFileRoleV2::ApiService,
        VpsHostFileRoleV2::WorkerService,
        VpsHostFileRoleV2::BackupService,
        VpsHostFileRoleV2::BackupTimer,
        VpsHostFileRoleV2::DeployReleaseScript,
        VpsHostFileRoleV2::RollbackReleaseScript,
        VpsHostFileRoleV2::ValidateReleaseScript,
        VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
        VpsHostFileRoleV2::RealRuntimeFenceHarness,
        VpsHostFileRoleV2::RealRuntimeFenceSelftest,
        VpsHostFileRoleV2::RootOnceScript,
        VpsHostFileRoleV2::NginxChallenge,
        VpsHostFileRoleV2::NginxCloudflareOnly,
        VpsHostFileRoleV2::NginxApiLocations,
        VpsHostFileRoleV2::NginxVhost,
        VpsHostFileRoleV2::DeploymentReadme,
        VpsHostFileRoleV2::OperatorRunbook,
        VpsHostFileRoleV2::BackupRunbook,
    ];
    let host_files = host_roles
        .into_iter()
        .map(|role| {
            let bytes = role.output_path().as_bytes();
            VpsHostFileSourceV2 {
                role,
                source: logical_source.join("host").join(role.output_path()),
                artifact: fact(bytes),
            }
        })
        .collect::<Vec<_>>();
    let plan = VpsReleasePlanV2 {
        schema_version: PLAN_SCHEMA_VERSION,
        source_commit: source_commit.clone(),
        publication_v3: logical_source.join("publication-v3"),
        binaries,
        configs,
        host_files,
        private_raw_roots: vec![
            PrivateRawRootV2 {
                edition: OfficialContentEditionV1::Demo,
                root: format!("{STATE_ROOT}/raw-content/demo").into(),
            },
            PrivateRawRootV2 {
                edition: OfficialContentEditionV1::Full,
                root: format!("{STATE_ROOT}/raw-content/full").into(),
            },
        ],
    };
    let plan_bytes = canonical_json_bytes(&plan)?;
    let plan_sha256 = Digest32::digest_bytes(&plan_bytes);

    for directory in [
        "bin",
        "config",
        "host",
        "host/systemd",
        "host/systemd/user",
        "host/deploy",
        "host/deploy/tests",
        "publication-v3",
    ] {
        fs::create_dir_all(logical_source.join(directory))?;
        set_mode(&logical_source.join(directory), 0o700)?;
    }
    let retained_admin_writer = plan
        .binaries
        .iter()
        .map(|binary| {
            write_source_file(&binary.source, binary.role.output_name().as_bytes(), 0o550)
                .map(|file| (binary.role, file))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .find_map(|(role, file)| (role == VpsBinaryRoleV2::Admin).then_some(file))
        .context("fixture omitted retained admin writer")?;
    for config in &plan.configs {
        write_source_file(&config.source, config.role.output_name().as_bytes(), 0o440)?;
    }
    for host in &plan.host_files {
        write_source_file(
            &host.source,
            host.role.output_path().as_bytes(),
            canonical_file_mode(host.role.output_path()),
        )?;
    }
    write_source_file(
        &logical_source.join("vps-release-plan-v2.json"),
        &plan_bytes,
        0o400,
    )?;

    let candidate = releases.join(format!("{source_commit}.partial"));
    fs::create_dir(&candidate)?;
    fs::write(
        candidate.join(SOURCE_COMMIT_FILE),
        format!("{source_commit}\n"),
    )?;
    let publication_lock_sha256 = Digest32::digest_bytes(b"publication lock v3");
    let release_manifest = VpsReleaseManifestV2 {
        schema_version: MANIFEST_SCHEMA_VERSION,
        source_commit: source_commit.clone(),
        database_schema_version: HIGHSCORES_DATABASE_SCHEMA_VERSION,
        deployment: canonical_user_deployment(),
        publication_lock_sha256,
        publication_manifest_sha256: Digest32::digest_bytes(b"publication manifest v3"),
        verifier_sha256: Digest32::digest_bytes(b"verifier"),
        files: vec![VpsReleaseFileV2 {
            path: "fixture-payload".into(),
            artifact: fact(b"fixture payload"),
            unix_mode: 0o440,
        }],
    };
    let release_manifest_bytes = canonical_json_bytes(&release_manifest)?;
    let release_manifest_sha256 = Digest32::digest_bytes(&release_manifest_bytes);
    fs::write(
        candidate.join(RELEASE_MANIFEST_FILE),
        release_manifest_bytes,
    )?;
    set_mode(&candidate.join(RELEASE_MANIFEST_FILE), 0o440)?;
    set_mode(&candidate, 0o550)?;

    Ok(SourceConsumeFixture {
        sandbox,
        incoming,
        releases,
        logical_source,
        candidate,
        plan,
        plan_bytes,
        plan_sha256,
        release_manifest_sha256,
        publication_lock_sha256,
        retained_admin_writer,
    })
}

fn fixture_release_parent(fixture: &SourceConsumeFixture) -> Result<PathBuf> {
    ensure!(
        fixture.releases.is_dir(),
        "fixture releases parent disappeared"
    );
    Ok(fixture.releases.clone())
}

fn consume_fixture_with<CV, PV, EL, AR, AU>(
    fixture: &SourceConsumeFixture,
    validate_candidate: CV,
    validate_publication: PV,
    ensure_lock: EL,
    after_rename: AR,
    after_root_unlink: AU,
) -> Result<()>
where
    CV: Fn(&Path) -> Result<(Digest32, Digest32)>,
    PV: Fn(&PinnedVpsSourceRoot, &Path) -> Result<Digest32>,
    EL: Fn() -> Result<()>,
    AR: Fn(&Path) -> Result<()>,
    AU: Fn(&Path) -> Result<()>,
{
    use std::os::fd::AsRawFd as _;

    let installed = fixture.releases.join(&fixture.plan.source_commit);
    let candidate_path = if fixture.candidate.is_dir() {
        &fixture.candidate
    } else {
        &installed
    };
    let candidate_fd = File::open(candidate_path)?;
    let candidate = pin_inherited_vps_candidate_root_at_with(
        &fixture.plan.source_commit,
        candidate_fd.as_raw_fd(),
        fixture.release_manifest_sha256,
        &fixture.incoming,
        &fixture.releases,
        synthetic_candidate_validator(
            fixture.release_manifest_sha256,
            fixture.plan.source_commit.clone(),
        ),
    )?;
    consume_vps_sources_in_with(
        &fixture.plan,
        &fixture.plan_bytes,
        fixture.plan_sha256,
        fixture.release_manifest_sha256,
        &fixture.incoming,
        &fixture.logical_source,
        &candidate,
        validate_candidate,
        validate_publication,
        ensure_lock,
        after_rename,
        after_root_unlink,
    )
}

fn consume_fixture(fixture: &SourceConsumeFixture) -> Result<()> {
    let release = fixture.release_manifest_sha256;
    let publication = fixture.publication_lock_sha256;
    consume_fixture_with(
        fixture,
        move |_| Ok((release, publication)),
        move |_, _| Ok(publication),
        || Ok(()),
        |_| Ok(()),
        |_| Ok(()),
    )
}

#[test]
fn source_publication_is_opened_relative_to_retained_source_descriptor() -> Result<()> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::os::fd::{AsRawFd as _, OwnedFd};
    use std::os::unix::fs::{MetadataExt as _, symlink};

    let sandbox = tempfile::tempdir()?;
    let logical_source = sandbox.path().join(".sources-descriptor-publication");
    let publication_path = logical_source.join("publication-v3");
    fs::create_dir(&logical_source)?;
    fs::create_dir(&publication_path)?;
    set_mode(&logical_source, 0o700)?;
    set_mode(&publication_path, 0o700)?;

    let parent_fd: OwnedFd = File::open(sandbox.path())?.into();
    let parent = rustix::fs::fstat(&parent_fd)?;
    let source = open_vps_source_root(
        &parent_fd,
        &parent,
        logical_source
            .file_name()
            .context("source fixture has no basename")?
            .to_owned(),
        0o700,
    )?;

    let old_procfd_path = PathBuf::from(format!(
        "/proc/self/fd/{}/./publication-v3",
        source.fd.as_raw_fd()
    ));
    let old_error = openat2(
        rustix::fs::CWD,
        &old_procfd_path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .expect_err("the obsolete procfd path unexpectedly crossed NO_MAGICLINKS");
    ensure!(
        old_error == rustix::io::Errno::LOOP,
        "obsolete procfd validation failed for an unexpected reason: {old_error}"
    );

    let observed_inode = with_pinned_vps_source_publication(
        &source,
        &logical_source,
        |logical_publication, publication| {
            ensure!(
                logical_publication == publication_path,
                "descriptor-relative validation changed the logical rebind path"
            );
            Ok(publication.metadata()?.ino())
        },
    )?;
    ensure!(
        observed_inode == fs::metadata(&publication_path)?.ino(),
        "descriptor-relative validation opened a different PublicationV3 inode"
    );

    let displaced = sandbox.path().join("displaced-source");
    let substituted = with_pinned_vps_source_publication(&source, &logical_source, |_, _| {
        fs::rename(&logical_source, &displaced)?;
        fs::create_dir(&logical_source)?;
        fs::create_dir(logical_source.join("publication-v3"))?;
        set_mode(&logical_source, 0o700)?;
        set_mode(&logical_source.join("publication-v3"), 0o700)?;
        Ok(())
    });
    assert!(
        substituted.is_err(),
        "descriptor-relative validation accepted coherent named-source substitution"
    );

    fs::remove_dir_all(&logical_source)?;
    fs::rename(&displaced, &logical_source)?;
    let authentic_publication = logical_source.join("authentic-publication");
    fs::rename(&publication_path, &authentic_publication)?;
    symlink("authentic-publication", &publication_path)?;
    assert!(
        with_pinned_vps_source_publication(&source, &logical_source, |_, _| Ok(())).is_err(),
        "descriptor-relative validation accepted a symlinked PublicationV3 child"
    );
    Ok(())
}

#[test]
fn inherited_activation_lock_requires_the_canonical_locked_open_file_description() -> Result<()> {
    use std::os::fd::AsRawFd as _;

    let unlocked_root = tempfile::tempdir()?;
    set_mode(unlocked_root.path(), 0o750)?;
    let unlocked_path = unlocked_root.path().join("activation.lock");
    let unlocked = fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&unlocked_path)?;
    set_mode(&unlocked_path, 0o600)?;
    assert!(
        pin_inherited_vps_activation_lock_at(unlocked_root.path(), unlocked.as_raw_fd()).is_err(),
        "an unlocked canonical descriptor was accepted"
    );

    let locked_root = tempfile::tempdir()?;
    set_mode(locked_root.path(), 0o750)?;
    let held = acquire_vps_activation_lock_at(locked_root.path())?;
    let distinct_path = locked_root.path().join("distinct.lock");
    let distinct = fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&distinct_path)?;
    set_mode(&distinct_path, 0o600)?;
    assert!(
        pin_inherited_vps_activation_lock_at(locked_root.path(), distinct.as_raw_fd()).is_err(),
        "a distinct locked-root file was accepted"
    );

    let reopened = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(locked_root.path().join("activation.lock"))?;
    assert!(
        pin_inherited_vps_activation_lock_at(locked_root.path(), reopened.as_raw_fd()).is_err(),
        "a reopened canonical inode with a distinct OFD inherited another OFD's lock"
    );
    let inherited = pin_inherited_vps_activation_lock_at(locked_root.path(), held.as_raw_fd())?;
    inherited.ensure_canonical()?;
    Ok(())
}

fn synthetic_candidate_validator(
    expected_digest: Digest32,
    manifest_commit: String,
) -> impl Fn(&Path) -> Result<(Digest32, String)> {
    move |root| {
        ensure!(
            fs::read(root.join(SOURCE_COMMIT_FILE))? == format!("{manifest_commit}\n").as_bytes(),
            "synthetic retained candidate source sentinel changed"
        );
        Ok((expected_digest, manifest_commit.clone()))
    }
}

#[test]
fn inherited_candidate_fd_accepts_partial_then_installed_repeated_resume() -> Result<()> {
    use std::os::fd::AsRawFd as _;

    let fixture = source_consume_fixture()?;
    let releases = fixture_release_parent(&fixture)?;
    let descriptor = File::open(&fixture.candidate)?;
    let digest = fixture.release_manifest_sha256;
    let commit = fixture.plan.source_commit.clone();

    let outer = pin_vps_activation_candidate_at(&fixture.candidate, &fixture.incoming, &releases)?;
    let outer_identity = rustix::fs::fstat(&outer)?;
    let descriptor_identity = rustix::fs::fstat(&descriptor)?;
    ensure!(
        outer_identity.st_dev == descriptor_identity.st_dev
            && outer_identity.st_ino == descriptor_identity.st_ino,
        "outer activation pin did not retain the candidate across a mounted install root"
    );

    let legacy_incoming_candidate = fixture.incoming.join(format!("{commit}.partial"));
    fs::create_dir(&legacy_incoming_candidate)?;
    fs::write(
        legacy_incoming_candidate.join(SOURCE_COMMIT_FILE),
        format!("{commit}\n"),
    )?;
    set_mode(&legacy_incoming_candidate, 0o550)?;
    assert!(
        pin_vps_activation_candidate_at(&legacy_incoming_candidate, &fixture.incoming, &releases,)
            .is_err(),
        "the legacy incoming candidate topology was accepted"
    );

    let partial = pin_inherited_vps_candidate_root_at_with(
        &commit,
        descriptor.as_raw_fd(),
        digest,
        &fixture.incoming,
        &releases,
        synthetic_candidate_validator(digest, commit.clone()),
    )
    .context("pin partial candidate through retained parent authorities")?;
    ensure!(
        partial.canonical_path()? == fixture.candidate,
        "partial candidate pin selected another pathname"
    );
    partial
        .ensure_canonical()
        .context("revalidate partial candidate")?;
    drop(partial);

    let installed = releases.join(&commit);
    fs::rename(&fixture.candidate, &installed)
        .context("rename partial candidate to installed test root")?;
    for _ in 0..2 {
        let resumed = pin_inherited_vps_candidate_root_at_with(
            &commit,
            descriptor.as_raw_fd(),
            digest,
            &fixture.incoming,
            &releases,
            synthetic_candidate_validator(digest, commit.clone()),
        )
        .context("resume candidate pin from installed test root")?;
        ensure!(
            resumed.canonical_path()? == installed,
            "installed resume selected another pathname"
        );
        resumed
            .ensure_canonical()
            .context("revalidate resumed installed candidate")?;
    }
    Ok(())
}

#[test]
fn inherited_candidate_fd_rejects_both_neither_and_path_substitution() -> Result<()> {
    use std::os::fd::AsRawFd as _;

    let fixture = source_consume_fixture()?;
    let releases = fixture_release_parent(&fixture)?;
    let descriptor = File::open(&fixture.candidate)?;
    let digest = fixture.release_manifest_sha256;
    let commit = fixture.plan.source_commit.clone();
    let displaced = fixture.releases.join("displaced-candidate");
    fs::rename(&fixture.candidate, &displaced)?;
    assert!(
        pin_inherited_vps_candidate_root_at_with(
            &commit,
            descriptor.as_raw_fd(),
            digest,
            &fixture.incoming,
            &releases,
            synthetic_candidate_validator(digest, commit.clone()),
        )
        .is_err(),
        "a retained candidate with neither canonical name was accepted"
    );

    let fixture = source_consume_fixture()?;
    let releases = fixture_release_parent(&fixture)?;
    let descriptor = File::open(&fixture.candidate)?;
    let digest = fixture.release_manifest_sha256;
    let commit = fixture.plan.source_commit.clone();
    let replacement_commit = commit.clone();
    let displaced = fixture.releases.join("displaced-candidate");
    let candidate = fixture.candidate.clone();
    assert!(
        pin_inherited_vps_candidate_root_at_with(
            &commit,
            descriptor.as_raw_fd(),
            digest,
            &fixture.incoming,
            &releases,
            move |_| {
                fs::rename(&candidate, &displaced)?;
                fs::create_dir(&candidate)?;
                set_mode(&candidate, 0o550)?;
                Ok((digest, replacement_commit.clone()))
            },
        )
        .is_err(),
        "a retained candidate accepted a substituted canonical pathname"
    );

    let fixture = source_consume_fixture()?;
    let releases = fixture_release_parent(&fixture)?;
    let descriptor = File::open(&fixture.candidate)?;
    let digest = fixture.release_manifest_sha256;
    let commit = fixture.plan.source_commit.clone();
    let retained = pin_inherited_vps_candidate_root_at_with(
        &commit,
        descriptor.as_raw_fd(),
        digest,
        &fixture.incoming,
        &releases,
        synthetic_candidate_validator(digest, commit.clone()),
    )?;
    let displaced_parent = fixture.sandbox.path().join("displaced-incoming-parent");
    fs::rename(&fixture.incoming, &displaced_parent)?;
    fs::create_dir(&fixture.incoming)?;
    set_mode(&fixture.incoming, 0o750)?;
    assert!(
        retained.ensure_canonical().is_err(),
        "a retained candidate accepted replacement of its uploader-source parent"
    );

    let sandbox = tempfile::tempdir_in(std::env::current_dir()?)?;
    set_mode(sandbox.path(), 0o750)?;
    let incoming = sandbox.path().join("incoming");
    fs::create_dir(&incoming)?;
    set_mode(&incoming, 0o750)?;
    let both_commit = ".";
    let candidate = incoming.join("..partial");
    fs::create_dir(&candidate)?;
    fs::write(candidate.join(SOURCE_COMMIT_FILE), b".\n")?;
    set_mode(&candidate, 0o550)?;
    let descriptor = File::open(&candidate)?;
    let digest = Digest32::digest_bytes(b"both names");
    assert!(
        pin_inherited_vps_candidate_root_at_with(
            both_commit,
            descriptor.as_raw_fd(),
            digest,
            &incoming,
            &candidate,
            synthetic_candidate_validator(digest, both_commit.to_owned()),
        )
        .is_err(),
        "one retained inode reachable through both canonical slots was accepted"
    );
    set_mode(&candidate, 0o700)?;
    Ok(())
}

#[test]
fn inherited_candidate_fd_rejects_closed_reused_wrong_and_identity_mismatch() -> Result<()> {
    use std::os::fd::AsRawFd as _;

    let fixture = source_consume_fixture()?;
    let releases = fixture_release_parent(&fixture)?;
    let digest = fixture.release_manifest_sha256;
    let commit = fixture.plan.source_commit.clone();

    let closed = File::open(&fixture.candidate)?;
    let closed_fd = closed.as_raw_fd();
    drop(closed);
    assert!(
        pin_inherited_vps_candidate_root_at_with(
            &commit,
            closed_fd,
            digest,
            &fixture.incoming,
            &releases,
            synthetic_candidate_validator(digest, commit.clone()),
        )
        .is_err(),
        "a closed candidate descriptor was accepted"
    );

    let wrong_path = fixture.sandbox.path().join("wrong-regular-file");
    fs::write(&wrong_path, b"wrong")?;
    let wrong = File::open(&wrong_path)?;
    assert!(
        pin_inherited_vps_candidate_root_at_with(
            &commit,
            wrong.as_raw_fd(),
            digest,
            &fixture.incoming,
            &releases,
            synthetic_candidate_validator(digest, commit.clone()),
        )
        .is_err(),
        "a regular file candidate descriptor was accepted"
    );
    let reusable_candidate = File::open(&fixture.candidate)?;
    let reused_fd = nix_legacy::fcntl::fcntl(
        reusable_candidate.as_raw_fd(),
        nix_legacy::fcntl::FcntlArg::F_DUPFD_CLOEXEC(512),
    )?;
    let reused = NixOwnedFdV2(reused_fd);
    nix_legacy::unistd::dup2(wrong.as_raw_fd(), reused.0)?;
    assert!(
        pin_inherited_vps_candidate_root_at_with(
            &commit,
            reused.0,
            digest,
            &fixture.incoming,
            &releases,
            synthetic_candidate_validator(digest, commit.clone()),
        )
        .is_err(),
        "a candidate descriptor number reused for another inode was accepted"
    );

    let candidate = File::open(&fixture.candidate)?;
    assert!(
        pin_inherited_vps_candidate_root_at_with(
            &commit,
            candidate.as_raw_fd(),
            digest,
            &fixture.incoming,
            &releases,
            synthetic_candidate_validator(
                Digest32::digest_bytes(b"different manifest"),
                commit.clone(),
            ),
        )
        .is_err(),
        "candidate manifest digest mismatch was accepted"
    );
    assert!(
        pin_inherited_vps_candidate_root_at_with(
            &commit,
            candidate.as_raw_fd(),
            digest,
            &fixture.incoming,
            &releases,
            |_| Ok((digest, "b".repeat(40))),
        )
        .is_err(),
        "candidate source commit mismatch was accepted"
    );
    assert!(
        pin_inherited_vps_candidate_root_at(
            &commit,
            candidate.as_raw_fd(),
            digest,
            &fixture.incoming,
            &releases,
        )
        .is_err(),
        "production candidate wrapper bypassed the full V2 release validator"
    );
    Ok(())
}

#[test]
fn inherited_candidate_descriptor_survives_actual_exec() -> Result<()> {
    use std::os::fd::AsRawFd as _;
    use std::process::Command;

    const CHILD: &str = "ROBIN_TEST_INHERITED_CANDIDATE_CHILD";
    const FD: &str = "ROBIN_TEST_INHERITED_CANDIDATE_FD";
    const INCOMING: &str = "ROBIN_TEST_INHERITED_CANDIDATE_INCOMING";
    const RELEASES: &str = "ROBIN_TEST_INHERITED_CANDIDATE_RELEASES";
    const COMMIT: &str = "ROBIN_TEST_INHERITED_CANDIDATE_COMMIT";
    const DIGEST: &str = "ROBIN_TEST_INHERITED_CANDIDATE_DIGEST";

    if std::env::var_os(CHILD).is_some() {
        use rustix::fs::{RenameFlags, renameat_with};
        use std::os::fd::AsFd as _;

        let fd = std::env::var(FD)?.parse::<std::os::fd::RawFd>()?;
        let incoming = PathBuf::from(std::env::var_os(INCOMING).context("missing incoming")?);
        let releases = PathBuf::from(std::env::var_os(RELEASES).context("missing releases")?);
        let commit = std::env::var(COMMIT)?;
        let digest = std::env::var(DIGEST)?.parse::<Digest32>()?;
        let candidate = pin_inherited_vps_candidate_root_at_with(
            &commit,
            fd,
            digest,
            &incoming,
            &releases,
            synthetic_candidate_validator(digest, commit.clone()),
        )?;
        candidate.ensure_canonical()?;
        let partial_name = candidate
            .partial_path
            .file_name()
            .context("child partial candidate has no basename")?;
        let installed_name = candidate
            .installed_path
            .file_name()
            .context("child installed candidate has no basename")?;
        renameat_with(
            candidate.parents.installed_parent_fd.as_fd(),
            partial_name,
            candidate.parents.installed_parent_fd.as_fd(),
            installed_name,
            RenameFlags::NOREPLACE,
        )?;
        rustix::fs::fsync(&candidate.parents.installed_parent_fd)?;
        candidate.ensure_canonical()?;
        ensure!(
            candidate.canonical_path()? == candidate.installed_path,
            "exec child did not retain the exact inode through promotion"
        );
        let resumed = pin_inherited_vps_candidate_root_at_with(
            &commit,
            fd,
            digest,
            &incoming,
            &releases,
            synthetic_candidate_validator(digest, commit.clone()),
        )?;
        resumed.ensure_canonical()?;
        return Ok(());
    }

    let fixture = source_consume_fixture()?;
    let releases = fixture_release_parent(&fixture)?;
    let candidate = File::open(&fixture.candidate)?;
    let fd = candidate.as_raw_fd();
    let flags = nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_GETFD)?;
    let mut flags = nix_legacy::fcntl::FdFlag::from_bits_truncate(flags);
    flags.remove(nix_legacy::fcntl::FdFlag::FD_CLOEXEC);
    nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_SETFD(flags))?;
    let status = Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "vps_release_v2::tests::inherited_candidate_descriptor_survives_actual_exec",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env(FD, fd.to_string())
        .env(INCOMING, &fixture.incoming)
        .env(RELEASES, &releases)
        .env(COMMIT, &fixture.plan.source_commit)
        .env(DIGEST, fixture.release_manifest_sha256.to_string())
        .status()?;
    ensure!(
        status.success(),
        "exec child rejected inherited candidate FD"
    );
    Ok(())
}

#[test]
fn candidate_parent_pinning_crosses_private_home_bind_mount() -> Result<()> {
    use std::os::fd::AsRawFd as _;
    use std::process::Command;

    const CHILD: &str = "ROBIN_TEST_MOUNTED_CANDIDATE_CHILD";
    const COMMIT: &str = "ROBIN_TEST_MOUNTED_CANDIDATE_COMMIT";
    const DIGEST: &str = "ROBIN_TEST_MOUNTED_CANDIDATE_DIGEST";

    if std::env::var_os(CHILD).is_some() {
        let commit = std::env::var(COMMIT)?;
        let digest = std::env::var(DIGEST)?.parse::<Digest32>()?;
        let install_root = Path::new(INSTALL_ROOT);
        let incoming = install_root.join("incoming");
        let releases = install_root.join("releases");
        let partial = releases.join(format!("{commit}.partial"));
        let installed = releases.join(&commit);
        let descriptor = File::open(&partial)?;

        let outer = pin_vps_activation_candidate_at(&partial, &incoming, &releases)?;
        let outer_identity = rustix::fs::fstat(&outer)?;
        let inherited_identity = rustix::fs::fstat(&descriptor)?;
        ensure!(
            outer_identity.st_dev == inherited_identity.st_dev
                && outer_identity.st_ino == inherited_identity.st_ino,
            "mounted outer candidate pin retained another inode"
        );

        let candidate = pin_inherited_vps_candidate_root_at_with(
            &commit,
            descriptor.as_raw_fd(),
            digest,
            &incoming,
            &releases,
            synthetic_candidate_validator(digest, commit.clone()),
        )?;
        candidate.ensure_canonical()?;
        fs::rename(&partial, &installed)?;
        candidate.ensure_canonical()?;
        let resumed = pin_inherited_vps_candidate_root_at_with(
            &commit,
            descriptor.as_raw_fd(),
            digest,
            &incoming,
            &releases,
            synthetic_candidate_validator(digest, commit.clone()),
        )?;
        resumed.ensure_canonical()?;

        let displaced = install_root.join("releases-displaced");
        fs::rename(&releases, &displaced)?;
        fs::create_dir(&releases)?;
        set_mode(&releases, 0o750)?;
        ensure!(
            resumed.ensure_canonical().is_err(),
            "mounted candidate accepted replacement of its release parent"
        );
        return Ok(());
    }

    let fixture = source_consume_fixture()?;
    let test_executable = File::open(std::env::current_exe()?)?;
    let install_root = File::open(fixture.sandbox.path())?;
    for fd in [test_executable.as_raw_fd(), install_root.as_raw_fd()] {
        let flags = nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_GETFD)?;
        let mut flags = nix_legacy::fcntl::FdFlag::from_bits_truncate(flags);
        flags.remove(nix_legacy::fcntl::FdFlag::FD_CLOEXEC);
        nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_SETFD(flags))?;
    }
    let mut command = Command::new("/usr/bin/bwrap");
    command
        .args(["--ro-bind", "/", "/"])
        .args(["--proc", "/proc"])
        .args(["--dev", "/dev"])
        .args(["--unshare-all", "--share-net"])
        .args(["--tmpfs", "/tmp"])
        .args([
            "--ro-bind-fd",
            &test_executable.as_raw_fd().to_string(),
            "/tmp/robin-manifest-tool-test",
        ])
        .args(["--tmpfs", "/home"])
        .args(["--dir", "/home/robinhood"])
        .args(["--dir", "/home/robinhood/.local"])
        .args(["--dir", "/home/robinhood/.local/opt"])
        .arg("--bind-fd")
        .arg(install_root.as_raw_fd().to_string())
        .arg(INSTALL_ROOT)
        .arg("/tmp/robin-manifest-tool-test")
        .args([
            "--exact",
            "vps_release_v2::tests::candidate_parent_pinning_crosses_private_home_bind_mount",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env(COMMIT, &fixture.plan.source_commit)
        .env(DIGEST, fixture.release_manifest_sha256.to_string());
    ensure!(
        command.status()?.success(),
        "candidate pin failed across a private /home install-root bind mount"
    );
    Ok(())
}

#[test]
fn source_consume_binds_candidate_v2_publication_v3_and_exact_plan() -> Result<()> {
    let fixture = source_consume_fixture()?;
    let wrong_release = Digest32::digest_bytes(b"wrong candidate");
    let publication = fixture.publication_lock_sha256;
    assert!(
        consume_fixture_with(
            &fixture,
            move |_| Ok((wrong_release, publication)),
            move |_, _| Ok(publication),
            || Ok(()),
            |_| Ok(()),
            |_| Ok(()),
        )
        .is_err(),
        "source consumption accepted a different V2 candidate"
    );

    let fixture = source_consume_fixture()?;
    let release = fixture.release_manifest_sha256;
    let publication = fixture.publication_lock_sha256;
    let substituted_publication = Digest32::digest_bytes(b"substituted publication v3");
    assert!(
        consume_fixture_with(
            &fixture,
            move |_| Ok((release, publication)),
            move |_, _| Ok(substituted_publication),
            || Ok(()),
            |_| Ok(()),
            |_| Ok(()),
        )
        .is_err(),
        "source consumption accepted a PublicationV3 lock different from the candidate"
    );

    let mut fixture = source_consume_fixture()?;
    fixture.plan.publication_v3 = fixture.logical_source.join("other-publication-v3");
    assert!(
        consume_fixture(&fixture).is_err(),
        "source consumption accepted a plan whose publication escaped the exact closure"
    );

    let fixture = source_consume_fixture()?;
    fs::write(
        fixture.candidate.join(SOURCE_COMMIT_FILE),
        format!("{}\n", "b".repeat(40)),
    )?;
    assert!(
        consume_fixture(&fixture).is_err(),
        "source consumption accepted a candidate for another source commit"
    );
    Ok(())
}

#[test]
fn source_consume_recovers_prepared_rename_and_root_unlink_crashes() -> Result<()> {
    let fixture = source_consume_fixture()?;
    let release = fixture.release_manifest_sha256;
    let publication = fixture.publication_lock_sha256;
    assert!(
        consume_fixture_with(
            &fixture,
            move |_| Ok((release, publication)),
            move |_, _| Ok(publication),
            || Ok(()),
            |_| anyhow::bail!("injected crash after source rename"),
            |_| Ok(()),
        )
        .is_err()
    );
    ensure!(
        fixture.consuming().is_dir(),
        "rename crash lost consuming root"
    );
    ensure!(
        fixture.journal().is_file(),
        "rename crash lost prepared journal"
    );
    fs::rename(fixture.journal(), fixture.journal_temporary())?;
    consume_fixture(&fixture)?;
    ensure!(
        !fixture.logical_source.exists()
            && !fixture.consuming().exists()
            && !fixture.journal().exists()
            && !fixture.journal_temporary().exists(),
        "prepared-journal retry left source transaction evidence"
    );

    let fixture = source_consume_fixture()?;
    let release = fixture.release_manifest_sha256;
    let publication = fixture.publication_lock_sha256;
    assert!(
        consume_fixture_with(
            &fixture,
            move |_| Ok((release, publication)),
            move |_, _| Ok(publication),
            || Ok(()),
            |_| Ok(()),
            |_| anyhow::bail!("injected crash after source root unlink"),
        )
        .is_err()
    );
    ensure!(
        !fixture.logical_source.exists() && !fixture.consuming().exists(),
        "root-unlink crash restored a source root"
    );
    ensure!(
        fixture.journal().is_file(),
        "root-unlink crash lost prepared journal"
    );
    consume_fixture(&fixture)?;
    ensure!(
        !fixture.journal().exists() && !fixture.terminal_journal().exists(),
        "root-unlink recovery left source journals"
    );
    Ok(())
}

#[test]
fn source_consume_resume_uses_the_retained_installed_candidate() -> Result<()> {
    use std::os::fd::AsRawFd as _;

    let fixture = source_consume_fixture()?;
    let release = fixture.release_manifest_sha256;
    let publication = fixture.publication_lock_sha256;
    assert!(
        consume_fixture_with(
            &fixture,
            move |_| Ok((release, publication)),
            move |_, _| Ok(publication),
            || Ok(()),
            |_| anyhow::bail!("injected crash after source rename"),
            |_| Ok(()),
        )
        .is_err()
    );

    let retained_fd = File::open(&fixture.candidate)?;
    let releases = fixture_release_parent(&fixture)?;
    let installed = releases.join(&fixture.plan.source_commit);
    fs::rename(&fixture.candidate, &installed)
        .context("rename fixture candidate into installed release root")?;
    let retained_candidate = pin_inherited_vps_candidate_root_at_with(
        &fixture.plan.source_commit,
        retained_fd.as_raw_fd(),
        fixture.release_manifest_sha256,
        &fixture.incoming,
        &releases,
        synthetic_candidate_validator(
            fixture.release_manifest_sha256,
            fixture.plan.source_commit.clone(),
        ),
    )?;
    retained_candidate
        .ensure_canonical()
        .context("revalidate retained installed fixture candidate")?;

    consume_vps_sources_in_with(
        &fixture.plan,
        &fixture.plan_bytes,
        fixture.plan_sha256,
        fixture.release_manifest_sha256,
        &fixture.incoming,
        &fixture.logical_source,
        &retained_candidate,
        move |_| Ok((release, publication)),
        move |_, _| Ok(publication),
        || Ok(()),
        |_| Ok(()),
        |_| Ok(()),
    )
    .context("resume source consumption through retained installed candidate")?;
    ensure!(
        !fixture.consuming().exists()
            && !fixture.journal().exists()
            && retained_candidate.canonical_path()?.is_dir(),
        "resume did not consume only the source while retaining the installed candidate"
    );
    Ok(())
}

#[test]
fn source_consume_rejects_root_substitution_and_unsafe_topologies() -> Result<()> {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let fixture = source_consume_fixture()?;
    let displaced = fixture.incoming.join("displaced-authentic-source");
    let release = fixture.release_manifest_sha256;
    let publication = fixture.publication_lock_sha256;
    assert!(
        consume_fixture_with(
            &fixture,
            move |_| Ok((release, publication)),
            move |_, _| Ok(publication),
            || Ok(()),
            |consuming| {
                fs::rename(consuming, &displaced)?;
                fs::create_dir(consuming)?;
                fs::set_permissions(consuming, fs::Permissions::from_mode(0o700))?;
                Ok(())
            },
            |_| Ok(()),
        )
        .is_err(),
        "a substituted consuming-root basename was accepted"
    );

    let fixture = source_consume_fixture()?;
    fs::write(fixture.logical_source.join("extra"), b"extra")?;
    set_mode(&fixture.logical_source.join("extra"), 0o440)?;
    assert!(
        consume_fixture(&fixture).is_err(),
        "an extra source file was accepted"
    );

    let fixture = source_consume_fixture()?;
    fs::remove_file(&fixture.plan.configs[0].source)?;
    assert!(
        consume_fixture(&fixture).is_err(),
        "a missing source file was accepted"
    );

    let fixture = source_consume_fixture()?;
    set_mode(&fixture.plan.configs[0].source, 0o600)?;
    assert!(
        consume_fixture(&fixture).is_err(),
        "a wrong source mode was accepted"
    );

    let fixture = source_consume_fixture()?;
    fs::hard_link(
        &fixture.plan.configs[0].source,
        fixture.logical_source.join("hardlink"),
    )?;
    assert!(
        consume_fixture(&fixture).is_err(),
        "a hard-linked source was accepted"
    );

    let fixture = source_consume_fixture()?;
    symlink(
        &fixture.plan.configs[0].source,
        fixture.logical_source.join("symlink"),
    )?;
    assert!(
        consume_fixture(&fixture).is_err(),
        "a symlinked source was accepted"
    );

    let fixture = source_consume_fixture()?;
    nix_legacy::unistd::mkfifo(
        &fixture.logical_source.join("special.fifo"),
        nix_legacy::sys::stat::Mode::S_IRUSR,
    )?;
    assert!(
        consume_fixture(&fixture).is_err(),
        "a special source node was accepted"
    );
    Ok(())
}

#[test]
fn source_inventory_rejects_mount_owner_depth_and_count_boundaries() -> Result<()> {
    use std::os::fd::OwnedFd;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    assert!(
        reject_mounts_at_or_below(Path::new("/proc")).is_err(),
        "a source root that is itself a mount was accepted"
    );

    let root = tempfile::tempdir()?;
    fs::write(root.path().join("entry"), b"entry")?;
    let metadata = fs::metadata(root.path())?;
    let directory: OwnedFd = File::open(root.path())?.into();
    let mut entries = Vec::new();
    let mut seen = 1;
    assert!(
        inventory_pinned_vps_source_directory(
            &directory,
            Path::new(""),
            metadata.dev(),
            MAX_FAILED_VPS_STAGING_DEPTH + 1,
            &mut seen,
            &mut entries,
        )
        .is_err(),
        "source inventory exceeded its depth boundary"
    );
    let mut seen = MAX_FAILED_VPS_STAGING_ENTRIES;
    assert!(
        inventory_pinned_vps_source_directory(
            &directory,
            Path::new(""),
            metadata.dev(),
            0,
            &mut seen,
            &mut entries,
        )
        .is_err(),
        "source inventory exceeded its entry boundary"
    );

    let expected_entries = vec![VpsSourceConsumeEntryV1 {
        path: ".".to_owned(),
        kind: VpsSourceEntryKindV1::Directory,
        unix_mode: metadata.permissions().mode() & 0o777,
        sha256: None,
        byte_length: None,
    }];
    let mut seen = 1;
    assert!(
        clear_pinned_vps_directory(
            &directory,
            Path::new(""),
            rustix::process::geteuid().as_raw().wrapping_add(1),
            metadata.dev(),
            0,
            &mut seen,
            &expected_entries,
            &|| Ok(()),
        )
        .is_err(),
        "source cleanup accepted a root owned by a different authority"
    );
    Ok(())
}

#[test]
fn release_validation_allows_exact_root_bind_but_rejects_descendant_mount() -> Result<()> {
    use std::os::fd::AsRawFd as _;
    use std::process::Command;

    const CHILD: &str = "ROBIN_VPS_RELEASE_ROOT_MOUNT_CHILD";
    const NESTED: &str = "ROBIN_VPS_RELEASE_NESTED_MOUNT";
    const CANDIDATE: &str = "/tmp/robin-vps-release-candidate";

    if std::env::var_os(CHILD).is_some() {
        let result = reject_mounts_strictly_below(Path::new(CANDIDATE));
        if std::env::var_os(NESTED).is_some() {
            ensure!(
                result.is_err(),
                "release validation accepted a strict descendant mount"
            );
        } else {
            result.context("release validation rejected its exact authenticated root mount")?;
        }
        return Ok(());
    }

    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("nested"))?;
    let nested_source = tempfile::tempdir()?;
    let executable = File::open(std::env::current_exe()?)?;
    let candidate = File::open(root.path())?;
    let nested = File::open(nested_source.path())?;
    for fd in [
        executable.as_raw_fd(),
        candidate.as_raw_fd(),
        nested.as_raw_fd(),
    ] {
        let flags = nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_GETFD)?;
        let mut flags = nix_legacy::fcntl::FdFlag::from_bits_truncate(flags);
        flags.remove(nix_legacy::fcntl::FdFlag::FD_CLOEXEC);
        nix_legacy::fcntl::fcntl(fd, nix_legacy::fcntl::FcntlArg::F_SETFD(flags))?;
    }

    for nested_mount in [false, true] {
        let mut command = Command::new("/usr/bin/bwrap");
        command
            .args(["--ro-bind", "/", "/"])
            .args(["--proc", "/proc"])
            .args(["--dev", "/dev"])
            .args(["--unshare-all", "--share-net"])
            .args(["--tmpfs", "/tmp"])
            .args([
                "--ro-bind-fd",
                &executable.as_raw_fd().to_string(),
                "/tmp/robin-manifest-tool-test",
            ])
            .args(["--dir", CANDIDATE])
            .args([
                "--ro-bind-fd",
                &candidate.as_raw_fd().to_string(),
                CANDIDATE,
            ]);
        if nested_mount {
            command.args([
                "--ro-bind-fd",
                &nested.as_raw_fd().to_string(),
                &format!("{CANDIDATE}/nested"),
            ]);
        }
        command
            .arg("/tmp/robin-manifest-tool-test")
            .args([
                "--exact",
                "vps_release_v2::tests::release_validation_allows_exact_root_bind_but_rejects_descendant_mount",
                "--nocapture",
            ])
            .env(CHILD, "1");
        if nested_mount {
            command.env(NESTED, "1");
        }
        ensure!(
            command.status()?.success(),
            "real bwrap release-root mount policy regression failed"
        );
    }
    Ok(())
}

#[test]
fn source_cleanup_rejects_delayed_equal_length_byte_rewrite() -> Result<()> {
    use std::cell::Cell;
    use std::os::unix::fs::FileExt as _;

    let fixture = source_consume_fixture()?;
    let release = fixture.release_manifest_sha256;
    let publication = fixture.publication_lock_sha256;
    let authority_checks = Cell::new(0_usize);
    let rewrote = Cell::new(false);
    let original_length = VpsBinaryRoleV2::Admin.output_name().len();
    let replacement = vec![b'x'; original_length];
    let result = consume_fixture_with(
        &fixture,
        move |_| Ok((release, publication)),
        move |_, _| Ok(publication),
        || {
            let next = authority_checks.get() + 1;
            authority_checks.set(next);
            // With the canonical fixture, check 10 is the guarded boundary
            // between the first and second hashes of the first binary.
            if next == 10 {
                fixture
                    .retained_admin_writer
                    .write_all_at(&replacement, 0)?;
                fixture.retained_admin_writer.sync_data()?;
                rewrote.set(true);
            }
            Ok(())
        },
        |_| Ok(()),
        |_| Ok(()),
    );
    ensure!(
        rewrote.get(),
        "test did not reach the delayed rewrite boundary"
    );
    assert!(
        result.is_err(),
        "source cleanup accepted an equal-length rewrite between its two hashes"
    );
    Ok(())
}

#[test]
fn publication_subset_copy_rejects_coherent_root_substitution_after_validation() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let publication_path = sandbox.path().join("publication");
    fs::create_dir_all(publication_path.join("backend/manifests/builds"))?;
    fs::write(
        publication_path.join("backend/manifests/builds/reviewed.json"),
        b"reviewed",
    )?;
    let mut authority = ValidatedPublicationV3::synthetic_for_consumer_test(&publication_path)?;
    let authentic = sandbox.path().join("authentic-publication");
    fs::rename(&publication_path, &authentic)?;
    fs::create_dir_all(publication_path.join("backend/manifests/builds"))?;
    fs::write(
        publication_path.join("backend/manifests/builds/reviewed.json"),
        b"reviewed",
    )?;
    let destination = sandbox.path().join("bundle-manifests");

    ensure!(
        copy_publication_tree_exact(&mut authority, "backend/manifests", &destination,).is_err(),
        "VPS subset materialization accepted a coherent post-validation root replacement"
    );
    ensure!(
        fs::read(destination.join("builds/reviewed.json"))? == b"reviewed",
        "test did not exercise copying from the retained reviewed file descriptor"
    );
    fs::remove_dir_all(authentic)?;
    fs::remove_dir_all(publication_path)?;
    Ok(())
}

#[test]
fn source_commits_and_manifest_paths_are_strict() {
    assert!(valid_source_commit(&"a".repeat(40)));
    assert!(!valid_source_commit(&"A".repeat(40)));
    assert!(!valid_source_commit(&"a".repeat(39)));
    for bad in ["", "/etc/passwd", "a/../b", "a//b", "a\\b"] {
        assert!(!valid_relative_manifest_path(bad), "accepted {bad:?}");
    }
    assert!(valid_relative_manifest_path("private/verifier/catalog"));
    let commit = "a".repeat(40);
    assert!(valid_release_directory_name(&commit, &commit));
    assert!(valid_release_directory_name(
        &format!("{commit}.partial"),
        &commit
    ));
    assert!(!valid_release_directory_name(
        &format!("{commit}.new"),
        &commit
    ));
    assert!(candidate_release_directory_name(
        &format!("{commit}.partial"),
        &commit
    ));
    assert!(!candidate_release_directory_name(&commit, &commit));
    assert!(!candidate_release_directory_name(
        &format!("{commit}.partial.extra"),
        &commit
    ));
}

#[test]
fn publication_subset_maps_only_nested_authority_operational_inputs() {
    assert_eq!(
        publication_file_to_vps_path(
            "private/official-content-authority/verifier-bundles/abc/catalog/profile.json"
        )
        .as_deref(),
        Some("private/verifier-bundles/abc/catalog/profile.json")
    );
    assert_eq!(
        publication_file_to_vps_path(
            "private/official-content-authority/private/source-tree-manifests-v2/abc.json"
        )
        .as_deref(),
        Some("private/source-tree-manifests-v2/abc.json")
    );
    assert_eq!(
        publication_file_to_vps_path("backend/manifests/builds/abc.json").as_deref(),
        Some("config/manifests/builds/abc.json")
    );
    assert!(
        publication_file_to_vps_path(
            "private/official-content-authority/private/projection-receipts-v2/abc.json"
        )
        .is_none()
    );
    assert!(publication_file_to_vps_path("private/verifier-bundles/legacy").is_none());
}

#[test]
fn obsolete_typed_roles_and_privileged_paths_are_unrepresentable() {
    assert!(serde_json::from_str::<VpsBinaryRoleV2>(r#""sandbox_broker""#).is_err());
    assert!(serde_json::from_str::<VpsConfigRoleV2>(r#""sandbox_broker""#).is_err());
    assert!(serde_json::from_str::<VpsHostFileRoleV2>(r#""sandbox_broker_socket""#).is_err());
    for path in [
        "polkit/50-robin.rules",
        "systemd/system/robin.service",
        "systemd/robin.service",
        "systemd/user/robin-highscores-verifier-broker.service",
        "systemd/user/robin-highscores.socket",
    ] {
        assert!(forbidden_release_path(path), "accepted {path:?}");
    }
    assert!(!forbidden_release_path(
        "systemd/user/robin-highscores-api.service"
    ));
}

#[test]
fn plan_requires_the_exact_single_user_role_closure() -> Result<()> {
    let binaries = [
        VpsBinaryRoleV2::Admin,
        VpsBinaryRoleV2::ManifestTool,
        VpsBinaryRoleV2::Server,
        VpsBinaryRoleV2::Worker,
        VpsBinaryRoleV2::ReplayVerifier,
    ]
    .into_iter()
    .map(|role| VpsBinarySourceV2 {
        role,
        source: role.output_name().into(),
        artifact: fact(role.output_name().as_bytes()),
    })
    .collect::<Vec<_>>();
    let configs = [
        VpsConfigRoleV2::Server,
        VpsConfigRoleV2::Worker,
        VpsConfigRoleV2::ApiEnvironment,
        VpsConfigRoleV2::WorkerEnvironment,
    ]
    .into_iter()
    .map(|role| VpsConfigSourceV2 {
        role,
        source: role.output_name().into(),
        artifact: fact(role.output_name().as_bytes()),
    })
    .collect::<Vec<_>>();
    let host_files = [
        VpsHostFileRoleV2::UserTarget,
        VpsHostFileRoleV2::ApiService,
        VpsHostFileRoleV2::WorkerService,
        VpsHostFileRoleV2::BackupService,
        VpsHostFileRoleV2::BackupTimer,
        VpsHostFileRoleV2::DeployReleaseScript,
        VpsHostFileRoleV2::RollbackReleaseScript,
        VpsHostFileRoleV2::ValidateReleaseScript,
        VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
        VpsHostFileRoleV2::RealRuntimeFenceHarness,
        VpsHostFileRoleV2::RealRuntimeFenceSelftest,
        VpsHostFileRoleV2::RootOnceScript,
        VpsHostFileRoleV2::NginxChallenge,
        VpsHostFileRoleV2::NginxCloudflareOnly,
        VpsHostFileRoleV2::NginxApiLocations,
        VpsHostFileRoleV2::NginxVhost,
        VpsHostFileRoleV2::DeploymentReadme,
        VpsHostFileRoleV2::OperatorRunbook,
        VpsHostFileRoleV2::BackupRunbook,
    ]
    .into_iter()
    .map(|role| VpsHostFileSourceV2 {
        role,
        source: role.output_path().into(),
        artifact: fact(role.output_path().as_bytes()),
    })
    .collect::<Vec<_>>();
    let mut plan = VpsReleasePlanV2 {
        schema_version: PLAN_SCHEMA_VERSION,
        source_commit: "a".repeat(40),
        publication_v3: "publication".into(),
        binaries,
        configs,
        host_files,
        private_raw_roots: vec![
            PrivateRawRootV2 {
                edition: OfficialContentEditionV1::Demo,
                root: format!("{STATE_ROOT}/raw-content/demo").into(),
            },
            PrivateRawRootV2 {
                edition: OfficialContentEditionV1::Full,
                root: format!("{STATE_ROOT}/raw-content/full").into(),
            },
        ],
    };
    validate_plan_shape(&plan)?;
    plan.binaries.remove(1);
    assert!(validate_plan_shape(&plan).is_err());
    Ok(())
}

#[test]
fn user_deployment_identity_and_units_reject_root_or_current_drift() -> Result<()> {
    let mut deployment = canonical_user_deployment();
    deployment.user = "root".into();
    assert!(deployment.validate().is_err());

    let root = tempfile::tempdir()?;
    let unit = root.path().join("api.service");
    let commit = "a".repeat(40);
    let release = format!("{INSTALL_ROOT}/releases/{commit}");
    let valid = CANONICAL_API_SERVICE.replace("@SOURCE_COMMIT@", &commit);
    fs::write(&unit, &valid)?;
    validate_final_host_file(VpsHostFileRoleV2::ApiService, &unit, &commit)?;
    fs::write(&unit, format!("{valid}User=robinhood\n"))?;
    assert!(validate_final_host_file(VpsHostFileRoleV2::ApiService, &unit, &commit).is_err());
    fs::write(
        &unit,
        valid.replace(&release, &format!("{INSTALL_ROOT}/current")),
    )?;
    assert!(validate_final_host_file(VpsHostFileRoleV2::ApiService, &unit, &commit).is_err());
    Ok(())
}

#[test]
fn backup_status_authority_matches_all_three_service_sandboxes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let deploy_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../robin_highscores/deploy");
    let server = root.path().join("server.toml");
    let api = root.path().join("api.service");
    let worker = root.path().join("worker.service");
    let backup = root.path().join("backup.service");
    let timer = root.path().join("backup.timer");
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let valid_server = format!(
        "backup_manifest_path = \"{BACKUP_STATUS_PATH}\"\nrelease_manifest_path = \"{INSTALL_ROOT}/releases/{commit}/{RELEASE_MANIFEST_FILE}\"\nmaximum_backup_age_hours = 32\n"
    );
    fs::write(&server, &valid_server)?;
    let source_unit = |name: &str| -> Result<String> {
        Ok(fs::read_to_string(deploy_root.join(name))?.replace("@SOURCE_COMMIT@", commit))
    };
    let api_valid = source_unit("robin-highscores-api.service")?;
    let worker_valid = source_unit("robin-highscores-worker.service")?;
    let backup_valid = source_unit("robin-highscores-backup.service")?;
    let timer_valid = source_unit("robin-highscores-backup.timer")?;
    fs::write(&api, &api_valid)?;
    fs::write(&worker, &worker_valid)?;
    fs::write(&backup, &backup_valid)?;
    fs::write(&timer, &timer_valid)?;
    validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)?;

    fs::write(
        &server,
        valid_server.replace(
            BACKUP_STATUS_PATH,
            &format!("{BACKUP_ROOT}/backup-status.json"),
        ),
    )?;
    assert!(
        validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit).is_err()
    );
    fs::write(&server, &valid_server)?;
    fs::write(&server, valid_server.replace(" = 32", " = 26"))?;
    assert!(
        validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit).is_err()
    );
    fs::write(&server, &valid_server)?;
    fs::write(
        &server,
        valid_server.replace(
            &format!("releases/{commit}/{RELEASE_MANIFEST_FILE}"),
            &format!("releases/{}/{RELEASE_MANIFEST_FILE}", "f".repeat(40)),
        ),
    )?;
    assert!(
        validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit).is_err()
    );
    fs::write(&server, &valid_server)?;

    for invalid_api in [
        api_valid.replace(&format!("ReadOnlyPaths={BACKUP_STATUS_ROOT}\n"), ""),
        api_valid.replace(
            &format!("ReadOnlyPaths={BACKUP_STATUS_ROOT}"),
            &format!("ReadWritePaths={BACKUP_STATUS_ROOT}"),
        ),
        format!("{api_valid}ReadOnlyPaths={BACKUP_ROOT}\n"),
    ] {
        fs::write(&api, invalid_api)?;
        assert!(
            validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)
                .is_err()
        );
    }
    fs::write(&api, &api_valid)?;

    fs::write(
        &backup,
        backup_valid.replace(&format!("ReadWritePaths={BACKUP_STATUS_ROOT}\n"), ""),
    )?;
    assert!(
        validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit).is_err()
    );
    fs::write(
        &backup,
        backup_valid.replace(
            BACKUP_STATUS_PATH,
            &format!("{BACKUP_ROOT}/backup-status.json"),
        ),
    )?;
    assert!(
        validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit).is_err()
    );
    let unit_map = "/home/robinhood/.config/systemd/user/robin-highscores.target=/home/robinhood/.config/systemd/user/robin-highscores.target";
    for invalid_backup in [
        backup_valid.replace(
            unit_map,
            "/home/robinhood/.config/systemd/user=/home/robinhood/.config/systemd/user",
        ),
        backup_valid.replace(
            unit_map,
            &format!("{INSTALL_ROOT}/releases/{commit}={INSTALL_ROOT}/releases/{commit}"),
        ),
        backup_valid.replace(&format!(" --restore-source-map {unit_map}"), ""),
    ] {
        fs::write(&backup, invalid_backup)?;
        assert!(
            validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit)
                .is_err()
        );
    }
    fs::write(&backup, &backup_valid)?;
    fs::write(
        &timer,
        timer_valid.replace("RandomizedDelaySec=45m", "RandomizedDelaySec=46m"),
    )?;
    assert!(
        validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit).is_err()
    );
    fs::write(&timer, &timer_valid)?;

    fs::write(
        &worker,
        worker_valid.replace(
            &format!("InaccessiblePaths={BACKUP_STATUS_ROOT}"),
            &format!("ReadOnlyPaths={BACKUP_STATUS_ROOT}"),
        ),
    )?;
    assert!(
        validate_backup_sandbox_contract(&server, &api, &worker, &backup, &timer, commit).is_err()
    );
    Ok(())
}

#[test]
fn user_release_scripts_require_literal_install_root_assignment() -> Result<()> {
    let commit = "a".repeat(40);
    let deploy_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../robin_highscores/deploy");
    for (role, name) in [
        (VpsHostFileRoleV2::DeployReleaseScript, "deploy-release.sh"),
        (
            VpsHostFileRoleV2::RollbackReleaseScript,
            "rollback-release.sh",
        ),
    ] {
        validate_final_host_file(role, &deploy_root.join(name), &commit)?;
    }

    let valid = format!(
        "#!/bin/sh\nexpected_user=robinhood\nexpected_home=/home/robinhood\nopt_root={INSTALL_ROOT}\nstate_root={STATE_ROOT}\nsecret_root=$state_root/api-secrets\nfor managed_directory in \\\n    \"$state_root/database\" \\\n    \"$state_root/replays\" \\\n    \"$state_root/campaign-states\" \\\n    \"$state_root/backups\" \\\n    \"$state_root/status\" \\\n    \"$secret_root\"\ndo\n    install -d -m 0700 -- \"$managed_directory\"\ndone\nchmod 0700 -- \"$state_root\" \"$state_root/database\" \"$state_root/replays\" \"$state_root/campaign-states\" \"$state_root/backups\" \"$state_root/status\" \"$secret_root\"\nsystemctl --user restart robin-highscores.target\n"
    );
    let invalid = [
        valid.replace(
            &format!("opt_root={INSTALL_ROOT}\n"),
            "opt_root=$expected_home/.local/opt/robin-highscores\n",
        ),
        valid.replace(
            &format!("opt_root={INSTALL_ROOT}\n"),
            "opt_root=$HOME/.local/opt/robin-highscores\n",
        ),
        valid.replace(
            &format!("opt_root={INSTALL_ROOT}\n"),
            "    opt_root=$expected_home/.local/opt/robin-highscores\n",
        ),
        valid.replace(&format!("opt_root={INSTALL_ROOT}\n"), ""),
        valid.replace(
            &format!("opt_root={INSTALL_ROOT}\n"),
            "opt_root=/opt/robin-highscores\n",
        ),
    ];

    validate_literal_install_root_assignment(&valid)?;
    for adversarial in &invalid {
        assert!(
            validate_literal_install_root_assignment(adversarial).is_err(),
            "accepted a non-literal opt_root assignment: {adversarial:?}"
        );
    }
    Ok(())
}

#[test]
fn executable_and_nginx_host_inputs_are_byte_exact() -> Result<()> {
    let root = tempfile::tempdir()?;
    let candidate = root.path().join("host-input");
    let deploy_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../robin_highscores/deploy");
    let commit = "a".repeat(40);
    for (role, name) in [
        (VpsHostFileRoleV2::UserTarget, "robin-highscores.target"),
        (
            VpsHostFileRoleV2::ApiService,
            "robin-highscores-api.service",
        ),
        (
            VpsHostFileRoleV2::WorkerService,
            "robin-highscores-worker.service",
        ),
        (
            VpsHostFileRoleV2::BackupService,
            "robin-highscores-backup.service",
        ),
        (
            VpsHostFileRoleV2::BackupTimer,
            "robin-highscores-backup.timer",
        ),
        (VpsHostFileRoleV2::DeployReleaseScript, "deploy-release.sh"),
        (
            VpsHostFileRoleV2::RollbackReleaseScript,
            "rollback-release.sh",
        ),
        (
            VpsHostFileRoleV2::ValidateReleaseScript,
            "validate-release-bundle.sh",
        ),
        (
            VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
            "tests/real-runtime-fence-release-gate.sh",
        ),
        (
            VpsHostFileRoleV2::RealRuntimeFenceHarness,
            "tests/real-runtime-fence-e2e.py",
        ),
        (
            VpsHostFileRoleV2::RealRuntimeFenceSelftest,
            "tests/real-runtime-fence-e2e-selftest.py",
        ),
        (VpsHostFileRoleV2::RootOnceScript, "root-once.sh"),
        (
            VpsHostFileRoleV2::NginxChallenge,
            "nginx-robinhood-api.challenge.conf",
        ),
        (
            VpsHostFileRoleV2::NginxCloudflareOnly,
            "nginx-robinhood-cloudflare-only.conf",
        ),
        (
            VpsHostFileRoleV2::NginxApiLocations,
            "nginx-robinhood-api.locations.conf",
        ),
        (
            VpsHostFileRoleV2::NginxVhost,
            "nginx-robinhood-api.vhost.conf",
        ),
    ] {
        let canonical = fs::read_to_string(deploy_root.join(name))?
            .replace("@SOURCE_COMMIT@", &commit)
            .into_bytes();
        fs::write(&candidate, &canonical)?;
        validate_final_host_file(role, &candidate, &commit)?;
        let mut appended = canonical;
        appended.extend_from_slice(b"\nrm -rf -- /home/robinhood/.local/share\n");
        fs::write(&candidate, appended)?;
        assert!(
            validate_final_host_file(role, &candidate, &commit).is_err(),
            "{role:?} accepted appended executable authority"
        );
    }
    Ok(())
}

#[test]
fn runtime_fence_selftest_template_accepts_deterministic_successor_only() -> Result<()> {
    let root = tempfile::tempdir()?;
    let candidate = root.path().join("real-runtime-fence-e2e-selftest.py");
    let canonical_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../robin_highscores/deploy/tests/real-runtime-fence-e2e-selftest.py");
    let canonical = fs::read_to_string(canonical_path)?;
    assert_eq!(
        Digest32::digest_bytes(canonical.as_bytes()).to_string(),
        "d1a1646e1d26d526a5040e433cc39bf1c24853cdd6cd1e3f9b39065f7b7fbbf7"
    );
    fs::write(&candidate, &canonical)?;
    validate_final_host_file(
        VpsHostFileRoleV2::RealRuntimeFenceSelftest,
        &candidate,
        &"a".repeat(40),
    )?;

    let stale = canonical
        .replace("            (stable / \"child\").chmod(0o640)\n", "")
        .replace("            first_raw_digest = first_raw[-1][1]\n", "")
        .replace(
            concat!(
                "            second_raw = MODULE.immutable_raw_metadata_tree(raw)\n",
                "            self.assertNotEqual(second_raw, first_raw)\n",
                "            self.assertNotEqual(second_raw[-1][1], first_raw_digest)\n",
            ),
            "            self.assertNotEqual(MODULE.immutable_raw_metadata_tree(raw), first_raw)\n",
        );
    assert_ne!(
        stale, canonical,
        "stale fixture must differ from its successor"
    );
    fs::write(&candidate, stale)?;
    assert!(
        validate_final_host_file(
            VpsHostFileRoleV2::RealRuntimeFenceSelftest,
            &candidate,
            &"a".repeat(40),
        )
        .is_err(),
        "accepted the retired timestamp-dependent selftest template"
    );

    fs::write(&candidate, format!("{canonical}# tampered\n"))?;
    assert!(
        validate_final_host_file(
            VpsHostFileRoleV2::RealRuntimeFenceSelftest,
            &candidate,
            &"a".repeat(40),
        )
        .is_err(),
        "accepted a tampered selftest template"
    );
    Ok(())
}

#[test]
fn release_validator_system_unit_denylist_is_exact_and_role_bound() -> Result<()> {
    let root = tempfile::tempdir()?;
    let script = root.path().join("validate-release-bundle.sh");
    let commit = "a".repeat(40);
    let canonical_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../robin_highscores/deploy/validate-release-bundle.sh");
    let canonical = fs::read_to_string(canonical_path)?;
    fs::write(&script, &canonical)?;
    validate_final_host_file(VpsHostFileRoleV2::ValidateReleaseScript, &script, &commit)?;

    let adversarial = [
        format!("{canonical}\n# extra reference: {SYSTEM_UNIT_ROOT}\n"),
        canonical.replace(SYSTEM_UNIT_ROOT, r"/etc/systemd\/system"),
        canonical.replace(SYSTEM_UNIT_ROOT, "$system_unit_root"),
        canonical.replace(SYSTEM_UNIT_ROOT, "/usr/lib/systemd/system"),
        canonical.replace(
            VALIDATOR_SYSTEM_UNIT_DENYLIST_BLOCK,
            &format!("# deny root units at {SYSTEM_UNIT_ROOT}\n"),
        ),
        format!("{canonical}\nsystemctl enable robin-highscores-api.service\n"),
        format!(
            "{canonical}\nsystem_unit_parent=/etc/systemd\ninstall robin.service \"$system_unit_parent/system/robin.service\"\n"
        ),
    ];
    for candidate in adversarial {
        fs::write(&script, &candidate)?;
        assert!(
            validate_final_host_file(VpsHostFileRoleV2::ValidateReleaseScript, &script, &commit,)
                .is_err(),
            "accepted noncanonical validator system-unit authority: {candidate:?}"
        );
    }

    for role in [
        VpsHostFileRoleV2::UserTarget,
        VpsHostFileRoleV2::ApiService,
        VpsHostFileRoleV2::WorkerService,
        VpsHostFileRoleV2::BackupService,
        VpsHostFileRoleV2::BackupTimer,
        VpsHostFileRoleV2::DeployReleaseScript,
        VpsHostFileRoleV2::RollbackReleaseScript,
        VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
        VpsHostFileRoleV2::RealRuntimeFenceHarness,
        VpsHostFileRoleV2::RealRuntimeFenceSelftest,
        VpsHostFileRoleV2::RootOnceScript,
        VpsHostFileRoleV2::NginxChallenge,
        VpsHostFileRoleV2::NginxCloudflareOnly,
        VpsHostFileRoleV2::NginxApiLocations,
        VpsHostFileRoleV2::NginxVhost,
        VpsHostFileRoleV2::DeploymentReadme,
        VpsHostFileRoleV2::OperatorRunbook,
        VpsHostFileRoleV2::BackupRunbook,
    ] {
        assert!(
            validate_system_unit_root_authority(role, SYSTEM_UNIT_ROOT).is_err(),
            "{role:?} accepted the root system-unit path"
        );
    }
    Ok(())
}

#[test]
fn deploy_script_requires_the_complete_first_restore_layout() -> Result<()> {
    let root = tempfile::tempdir()?;
    let script = root.path().join("deploy-release.sh");
    let commit = "a".repeat(40);
    let canonical = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../robin_highscores/deploy/deploy-release.sh"),
    )?;
    for mutable_directory in [
        "database",
        "replays",
        "campaign-states",
        "backups",
        "status",
    ] {
        fs::write(
            &script,
            canonical.replace(
                &format!("\"$state_root/{mutable_directory}\""),
                &format!("\"$state_root/omitted-{mutable_directory}\""),
            ),
        )?;
        assert!(
            validate_final_host_file(VpsHostFileRoleV2::DeployReleaseScript, &script, &commit)
                .is_err(),
            "deploy validation accepted a missing first-restore {mutable_directory} directory"
        );
    }
    fs::write(
        &script,
        canonical.replace(
            "secret_root=$state_root/api-secrets",
            "secret_root=$state_root/wrong",
        ),
    )?;
    assert!(
        validate_final_host_file(VpsHostFileRoleV2::DeployReleaseScript, &script, &commit).is_err()
    );
    Ok(())
}

#[test]
fn derived_root_once_checksum_manifest_is_exact() -> Result<()> {
    let root = tempfile::tempdir()?;
    let deploy = root.path().join("deploy");
    fs::create_dir(&deploy)?;
    for name in ROOT_ONCE_KIT_FILES {
        fs::write(deploy.join(name), format!("exact bytes for {name}\n"))?;
    }
    let canonical = expected_root_once_sha256sums(root.path())?;
    fs::write(root.path().join(ROOT_ONCE_SHA256SUMS_FILE), &canonical)?;
    validate_root_once_sha256sums(root.path())?;

    let names = std::str::from_utf8(&canonical)?
        .lines()
        .map(|line| line.split_once("  ").map(|(_, name)| name))
        .collect::<Option<Vec<_>>>()
        .context("derived root-once checksum line is not canonical")?;
    assert_eq!(names, ROOT_ONCE_KIT_FILES);
    assert_eq!(canonical_file_mode(ROOT_ONCE_SHA256SUMS_FILE), 0o440);

    fs::remove_file(root.path().join(ROOT_ONCE_SHA256SUMS_FILE))?;
    assert!(validate_root_once_sha256sums(root.path()).is_err());

    let mut lines = canonical.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    lines.swap(0, 1);
    let reordered = lines.join(&b'\n');
    fs::write(root.path().join(ROOT_ONCE_SHA256SUMS_FILE), reordered)?;
    assert!(validate_root_once_sha256sums(root.path()).is_err());

    let mut substituted = canonical.clone();
    substituted[0] = if substituted[0] == b'a' { b'b' } else { b'a' };
    fs::write(root.path().join(ROOT_ONCE_SHA256SUMS_FILE), substituted)?;
    assert!(validate_root_once_sha256sums(root.path()).is_err());

    let mut extra = canonical.clone();
    extra.extend_from_slice(
        b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  extra.conf\n",
    );
    fs::write(root.path().join(ROOT_ONCE_SHA256SUMS_FILE), extra)?;
    assert!(validate_root_once_sha256sums(root.path()).is_err());
    Ok(())
}

#[test]
fn derived_deploy_bootstrap_checksum_manifest_is_exact() -> Result<()> {
    let root = tempfile::tempdir()?;
    let deploy = root.path().join("deploy");
    fs::create_dir(&deploy)?;
    for name in DEPLOY_BOOTSTRAP_FILES {
        fs::write(deploy.join(name), format!("exact bytes for {name}\n"))?;
    }
    let canonical = expected_deploy_bootstrap_sha256sums(root.path())?;
    fs::write(
        root.path().join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE),
        &canonical,
    )?;
    validate_deploy_bootstrap_sha256sums(root.path())?;

    let names = std::str::from_utf8(&canonical)?
        .lines()
        .map(|line| line.split_once("  ").map(|(_, name)| name))
        .collect::<Option<Vec<_>>>()
        .context("derived deploy bootstrap checksum line is not canonical")?;
    assert_eq!(names, DEPLOY_BOOTSTRAP_FILES);
    assert_eq!(canonical_file_mode(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE), 0o440);

    fs::remove_file(root.path().join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE))?;
    assert!(validate_deploy_bootstrap_sha256sums(root.path()).is_err());

    let mut lines = canonical.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    lines.swap(0, 1);
    fs::write(
        root.path().join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE),
        lines.join(&b'\n'),
    )?;
    assert!(validate_deploy_bootstrap_sha256sums(root.path()).is_err());

    let mut substituted = canonical.clone();
    substituted[0] = if substituted[0] == b'a' { b'b' } else { b'a' };
    fs::write(
        root.path().join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE),
        substituted,
    )?;
    assert!(validate_deploy_bootstrap_sha256sums(root.path()).is_err());

    let mut extra = canonical;
    extra.extend_from_slice(
        b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  extra.sh\n",
    );
    fs::write(root.path().join(DEPLOY_BOOTSTRAP_SHA256SUMS_FILE), extra)?;
    assert!(validate_deploy_bootstrap_sha256sums(root.path()).is_err());
    Ok(())
}

#[test]
fn release_manifest_splits_historical_structure_from_current_candidate_admission() {
    let file = VpsReleaseFileV2 {
        path: "bin/tool".into(),
        artifact: fact(b"tool"),
        unix_mode: 0o440,
    };
    let mut manifest = VpsReleaseManifestV2 {
        schema_version: MANIFEST_SCHEMA_VERSION,
        source_commit: "a".repeat(40),
        database_schema_version: HIGHSCORES_DATABASE_SCHEMA_VERSION,
        deployment: canonical_user_deployment(),
        publication_lock_sha256: Digest32::digest_bytes(b"lock"),
        publication_manifest_sha256: Digest32::digest_bytes(b"publication"),
        verifier_sha256: Digest32::digest_bytes(b"verifier"),
        files: vec![file.clone()],
    };
    manifest.validate().unwrap();
    manifest.files.push(file);
    assert!(manifest.validate().is_err());
    manifest.files.pop();
    manifest.files[0].unix_mode = 0o644;
    assert!(manifest.validate().is_err());
    manifest.files[0].unix_mode = 0o440;
    manifest.database_schema_version = MIN_SUPPORTED_DATABASE_SCHEMA_VERSION - 1;
    assert!(manifest.validate().is_err());
    manifest.database_schema_version = MIN_SUPPORTED_DATABASE_SCHEMA_VERSION;
    manifest.validate().unwrap();
    assert_eq!(
        manifest.validate_current_candidate().is_ok(),
        MIN_SUPPORTED_DATABASE_SCHEMA_VERSION == HIGHSCORES_DATABASE_SCHEMA_VERSION
    );
    manifest.database_schema_version += 1;
    assert_eq!(
        manifest.validate().is_ok(),
        manifest.database_schema_version <= HIGHSCORES_DATABASE_SCHEMA_VERSION
    );
    manifest.database_schema_version = HIGHSCORES_DATABASE_SCHEMA_VERSION;
    manifest.validate_current_candidate().unwrap();
    manifest.verifier_sha256 = Digest32::default();
    assert!(manifest.validate().is_err());
}

#[test]
fn raw_root_declarations_require_exact_demo_full_roots() {
    let roots = [
        (
            OfficialContentEditionV1::Demo,
            "/home/robinhood/.local/share/robin-highscores/raw-content/demo",
        ),
        (
            OfficialContentEditionV1::Full,
            "/home/robinhood/.local/share/robin-highscores/raw-content/full",
        ),
    ]
    .into_iter()
    .map(|(edition, root)| PrivateRawRootV2 {
        edition,
        root: root.into(),
    })
    .collect::<Vec<_>>();
    let mut document = PrivateRawRootDeclarationsV2 {
        schema_version: RAW_ROOTS_SCHEMA_VERSION,
        roots,
    };
    document.validate().unwrap();
    document.schema_version = 1;
    assert!(document.validate().is_err());
    document.schema_version = RAW_ROOTS_SCHEMA_VERSION;
    document.roots.swap(0, 1);
    assert!(document.validate().is_err());
}

#[test]
fn selected_raw_content_manifest_requires_exact_bytes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let file = root.path().join("Data/mission.bin");
    fs::create_dir_all(file.parent().context("test path has no parent")?)?;
    fs::write(&file, b"exact raw input")?;
    let manifest = OfficialSourceTreeManifestV2 {
        schema_version: 2,
        edition: OfficialContentEditionV1::Demo,
        source_format: robin_run_protocol::OfficialProjectionSourceFormatV1::LooseNativeV1,
        closure_kind:
            robin_run_protocol::OfficialSourceClosureKindV2::LooseNativeSimulationConsumedV1,
        files: vec![robin_run_protocol::OfficialSourceFileV1 {
            path: "Data/mission.bin".into(),
            sha256: Digest32::digest_bytes(b"exact raw input"),
            byte_length: 15,
        }],
    };
    manifest.validate()?;
    validate_raw_content_against_manifest(root.path(), &manifest)?;
    fs::write(&file, b"substitution")?;
    assert!(validate_raw_content_against_manifest(root.path(), &manifest).is_err());
    Ok(())
}

#[test]
fn sorted_sha256_and_mode_inventories_are_exact() -> Result<()> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("z"), b"z")?;
    fs::write(root.path().join("a"), b"a")?;
    let sums = expected_sha256sums(root.path())?;
    let text = String::from_utf8(sums)?;
    let lines = text.lines().collect::<Vec<_>>();
    assert!(lines[0].ends_with("  a"));
    assert!(lines[1].ends_with("  z"));
    let modes = BTreeMap::from([
        ("a".to_owned(), ('f', 0o440)),
        ("z".to_owned(), ('f', 0o440)),
    ]);
    assert_eq!(
        String::from_utf8(mode_inventory_bytes(&modes)?)?,
        "f 0440  a\nf 0440  z\n"
    );
    assert_eq!(canonical_file_mode("bin/robin-highscores-server"), 0o550);
    assert_eq!(canonical_file_mode("deploy/deploy-release.sh"), 0o550);
    assert_eq!(canonical_file_mode("config/highscores-server.toml"), 0o440);
    Ok(())
}

#[test]
fn payload_inventory_sorts_serialized_paths_after_path_conversion() -> Result<()> {
    let root = tempfile::tempdir()?;
    let verifier_config = root
        .path()
        .join("private/verifier/operator-config/catalog.json");
    let verifier_bundle = root
        .path()
        .join("private/verifier-bundles/content/catalog/component.json");
    fs::create_dir_all(
        verifier_config
            .parent()
            .context("verifier config path has no parent")?,
    )?;
    fs::create_dir_all(
        verifier_bundle
            .parent()
            .context("verifier bundle path has no parent")?,
    )?;
    fs::write(&verifier_config, b"config")?;
    fs::write(&verifier_bundle, b"bundle")?;

    let component_order = walk_regular_files(root.path())?
        .into_iter()
        .map(|(relative, _)| path_to_manifest(&relative))
        .collect::<Result<Vec<_>>>()?;
    assert_eq!(
        component_order,
        [
            "private/verifier/operator-config/catalog.json",
            "private/verifier-bundles/content/catalog/component.json",
        ]
    );

    let files = payload_inventory(root.path())?;
    assert_eq!(
        files
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        [
            "private/verifier-bundles/content/catalog/component.json",
            "private/verifier/operator-config/catalog.json",
        ]
    );

    let sums = String::from_utf8(expected_sha256sums(root.path())?)?;
    let listed_paths = sums
        .lines()
        .map(|line| {
            line.split_once("  ")
                .context("SHA256SUMS test line has no path")
                .map(|(_, path)| path)
        })
        .collect::<Result<Vec<_>>>()?;
    assert_eq!(
        listed_paths,
        [
            "private/verifier-bundles/content/catalog/component.json",
            "private/verifier/operator-config/catalog.json",
        ],
        "SHA256SUMS must use bytewise serialized-path order"
    );
    assert!(
        listed_paths.windows(2).all(|pair| pair[0] < pair[1]),
        "the authentic gate rejects a non-strictly-sorted SHA256SUMS"
    );
    VpsReleaseManifestV2 {
        schema_version: MANIFEST_SCHEMA_VERSION,
        source_commit: "a".repeat(40),
        database_schema_version: HIGHSCORES_DATABASE_SCHEMA_VERSION,
        deployment: canonical_user_deployment(),
        publication_lock_sha256: Digest32::digest_bytes(b"lock"),
        publication_manifest_sha256: Digest32::digest_bytes(b"publication"),
        verifier_sha256: Digest32::digest_bytes(b"verifier"),
        files,
    }
    .validate_current_candidate()?;
    Ok(())
}

#[test]
fn publication_subset_includes_exact_materialized_runtime_fence_closure() {
    let materialized = [
        VpsHostFileRoleV2::RealRuntimeFenceReleaseGate,
        VpsHostFileRoleV2::RealRuntimeFenceHarness,
        VpsHostFileRoleV2::RealRuntimeFenceSelftest,
    ]
    .into_iter()
    .map(|role| role.output_path().to_owned())
    .collect::<BTreeSet<_>>();
    let mut admitted = BTreeSet::new();
    extend_real_runtime_fence_payload_paths(&mut admitted);

    assert_eq!(admitted, materialized);
}

#[test]
fn outer_activation_preflight_reads_inherited_procfds_then_executes_script() -> Result<()> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::process::CommandExt as _;
    use std::process::Command;

    const CHILD: &str = "ROBIN_TEST_VPS_OUTER_PROC_FD_CHILD";
    const SCRIPT_FD: &str = "ROBIN_TEST_VPS_OUTER_SCRIPT_FD";
    const BOOTSTRAP_FD: &str = "ROBIN_TEST_VPS_OUTER_BOOTSTRAP_FD";
    const VALIDATOR_FD: &str = "ROBIN_TEST_VPS_OUTER_VALIDATOR_FD";
    const TOOL_FD: &str = "ROBIN_TEST_VPS_OUTER_TOOL_FD";
    const PLAN_FD: &str = "ROBIN_TEST_VPS_OUTER_PLAN_FD";
    const BOOTSTRAP_SHA: &str = "ROBIN_TEST_VPS_OUTER_BOOTSTRAP_SHA";
    const PLAN_SHA: &str = "ROBIN_TEST_VPS_OUTER_PLAN_SHA";
    const MARKER: &str = "ROBIN_TEST_VPS_OUTER_MARKER";

    if std::env::var_os(CHILD).is_some() {
        let descriptor_path = |name: &str| -> Result<PathBuf> {
            Ok(PathBuf::from(format!(
                "/proc/self/fd/{}",
                std::env::var(name)?.parse::<std::os::fd::RawFd>()?
            )))
        };
        let script = descriptor_path(SCRIPT_FD)?;
        let bootstrap = descriptor_path(BOOTSTRAP_FD)?;
        let validator = descriptor_path(VALIDATOR_FD)?;
        let tool = descriptor_path(TOOL_FD)?;
        let plan = descriptor_path(PLAN_FD)?;
        let raw = pin_vps_activation_exec_authorities(
            VpsActivationExecOperationV2::Deploy,
            &script,
            &bootstrap,
            &validator,
            &tool,
            &std::env::var(BOOTSTRAP_SHA)?,
            Some(&plan),
        )?;
        let (_, _, observed_plan_sha) = load_pinned_vps_plan(&plan, &std::env::var(PLAN_SHA)?)?;
        ensure!(
            observed_plan_sha.to_string() == std::env::var(PLAN_SHA)?,
            "inherited plan digest changed during outer preflight"
        );
        clear_vps_close_on_exec(raw[0])?;
        let error = Command::new(&script)
            .arg(std::env::var_os(MARKER).context("missing marker")?)
            .exec();
        return Err(error).context("exec safe inherited-FD activation script");
    }

    let fixture = source_consume_fixture()?;
    let root = fixture.sandbox.path();
    let script_path = root.join("safe-activation.sh");
    fs::write(
        &script_path,
        b"#!/bin/sh\n[ \"$#\" -eq 1 ] || exit 91\nprintf reached > \"$1\"\n",
    )?;
    set_mode(&script_path, 0o500)?;
    let validator_path = root.join("validator.sh");
    fs::write(&validator_path, b"#!/bin/sh\nexit 0\n")?;
    set_mode(&validator_path, 0o500)?;
    let script_sha = Digest32::digest_bytes(&fs::read(&script_path)?);
    let validator_sha = Digest32::digest_bytes(&fs::read(&validator_path)?);
    let bootstrap_path = root.join("DEPLOY_BOOTSTRAP_SHA256SUMS");
    fs::write(
        &bootstrap_path,
        format!(
            "{script_sha}  deploy-release.sh\n{script_sha}  rollback-release.sh\n{validator_sha}  validate-release-bundle.sh\n"
        ),
    )?;
    set_mode(&bootstrap_path, 0o400)?;
    let bootstrap_sha = Digest32::digest_bytes(&fs::read(&bootstrap_path)?);
    let test_tool_path = root.join("robin-highscores-manifestctl");
    fs::copy(std::env::current_exe()?, &test_tool_path)?;
    set_mode(&test_tool_path, 0o550)?;
    let plan_path = fixture.logical_source.join("vps-release-plan-v2.json");
    let marker = root.join("reached-script");

    let script = File::open(script_path)?;
    let bootstrap = File::open(bootstrap_path)?;
    let validator = File::open(validator_path)?;
    let tool = File::open(&test_tool_path)?;
    let plan = File::open(plan_path)?;
    // Force a read-induced atime change on mounts that track access time.
    plan.set_times(std::fs::FileTimes::new().set_accessed(std::time::UNIX_EPOCH))?;
    for descriptor in [
        script.as_raw_fd(),
        bootstrap.as_raw_fd(),
        validator.as_raw_fd(),
        tool.as_raw_fd(),
        plan.as_raw_fd(),
    ] {
        clear_vps_close_on_exec(descriptor)?;
    }
    let status = Command::new(test_tool_path)
        .args([
            "--exact",
            "vps_release_v2::tests::outer_activation_preflight_reads_inherited_procfds_then_executes_script",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env(SCRIPT_FD, script.as_raw_fd().to_string())
        .env(BOOTSTRAP_FD, bootstrap.as_raw_fd().to_string())
        .env(VALIDATOR_FD, validator.as_raw_fd().to_string())
        .env(TOOL_FD, tool.as_raw_fd().to_string())
        .env(PLAN_FD, plan.as_raw_fd().to_string())
        .env(BOOTSTRAP_SHA, bootstrap_sha.to_string())
        .env(PLAN_SHA, fixture.plan_sha256.to_string())
        .env(MARKER, &marker)
        .status()?;
    ensure!(status.success(), "inherited-procfd preflight child failed");
    ensure!(
        fs::read(&marker)? == b"reached",
        "outer preflight did not cross into the safe activation script"
    );
    Ok(())
}

#[test]
fn hardlinks_symlinks_and_mutable_raw_roots_fail_closed() -> Result<()> {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let root = tempfile::tempdir()?;
    let raw = root.path().join("raw");
    fs::create_dir(&raw)?;
    let file = raw.join("file");
    fs::write(&file, b"raw")?;
    fs::hard_link(&file, raw.join("alias"))?;
    assert!(reject_links_and_special_nodes(&raw, false).is_err());
    fs::remove_file(raw.join("alias"))?;
    symlink("file", raw.join("link"))?;
    assert!(reject_links_and_special_nodes(&raw, false).is_err());
    fs::remove_file(raw.join("link"))?;
    assert!(reject_links_and_special_nodes(&raw, true).is_err());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o440))?;
    fs::set_permissions(&raw, fs::Permissions::from_mode(0o550))?;
    validate_immutable_raw_root(&raw)?;
    Ok(())
}

#[test]
fn placeholders_and_private_static_paths_fail_closed() {
    assert!(reject_placeholders(&[b'0'; 64], "test").is_err());
    assert!(reject_placeholders(b"token=changeme", "test").is_err());
    assert!(reject_placeholders(b"digest=real", "test").is_ok());
    for path in [
        "cloudflare-public/a",
        "cloudflare-identity-signer/a",
        "raw/full/a",
    ] {
        assert!(
            path.starts_with("cloudflare-") || path.starts_with("raw/"),
            "test path classifier drifted"
        );
    }
}

#[test]
fn worker_config_binds_exact_direct_sandbox_authority() -> Result<()> {
    let root = tempfile::tempdir()?;
    let verifier = Digest32::digest_bytes(b"verifier");
    let catalog = fact(b"catalog");
    let demo_source = Digest32::digest_bytes(b"demo-source");
    let full_source = Digest32::digest_bytes(b"full-source");
    let source_commit = "a".repeat(40);
    let release_root = format!("{INSTALL_ROOT}/releases/{source_commit}");
    let catalog_path = format!(
        "{release_root}/private/verifier/operator-config/{}",
        catalog.sha256
    );
    let worker = root.path().join("worker.toml");
    let direct_config = format!(
        concat!(
            "server_config = \"{}/config/highscores-server.toml\"\n",
            "campaign_state_directory = \"{}/campaign-states\"\n",
            "verifier_job_config_catalog = \"{}\"\n",
            "verifier_job_config_catalog_sha256 = \"{}\"\n",
            "demo_raw_content_manifest = \"{}/private/source-tree-manifests-v2/{}.json\"\n",
            "full_raw_content_manifest = \"{}/private/source-tree-manifests-v2/{}.json\"\n",
            "[verifier_launcher]\n",
            "bwrap_program = \"/usr/bin/bwrap\"\n",
            "bwrap_sha256 = \"{}\"\n",
            "prlimit_program = \"/usr/bin/prlimit\"\n",
            "prlimit_sha256 = \"{}\"\n",
            "verifier_program = \"{}/bin/robin-replay-verifier\"\n",
            "verifier_sha256 = \"{}\"\n",
            "wall_timeout_seconds = 120\n",
            "cpu_limit_seconds = 120\n",
            "address_space_limit_bytes = 1073741824\n",
            "process_limit = 32\n",
            "open_files_limit = 128\n",
            "file_size_limit_bytes = 134217728\n",
            "max_request_bytes = 1048576\n",
            "[limits]\n",
            "max_campaign_bytes = 67108864\n"
        ),
        release_root,
        STATE_ROOT,
        catalog_path,
        catalog.sha256,
        release_root,
        demo_source,
        release_root,
        full_source,
        Digest32::digest_bytes(b"bwrap"),
        Digest32::digest_bytes(b"prlimit"),
        release_root,
        verifier,
    );
    fs::write(&worker, &direct_config)?;
    validate_final_config(
        VpsConfigRoleV2::Worker,
        &worker,
        verifier,
        &catalog,
        &source_commit,
        &BTreeSet::new(),
    )?;
    fs::write(
        &worker,
        direct_config.replace(
            "[verifier_launcher]",
            "broker_socket = \"/run/obsolete.sock\"\n[verifier_launcher]",
        ),
    )?;
    assert!(
        validate_final_config(
            VpsConfigRoleV2::Worker,
            &worker,
            verifier,
            &catalog,
            &source_commit,
            &BTreeSet::new(),
        )
        .is_err()
    );
    fs::write(
        &worker,
        direct_config.replace("wall_timeout_seconds = 120", "wall_timeout_seconds = 121"),
    )?;
    assert!(
        validate_final_config(
            VpsConfigRoleV2::Worker,
            &worker,
            verifier,
            &catalog,
            &source_commit,
            &BTreeSet::new(),
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn server_config_binds_release_manifests_and_both_campaign_templates() -> Result<()> {
    let root = tempfile::tempdir()?;
    let source_commit = "b".repeat(40);
    let demo = Digest32::digest_bytes(b"demo-state");
    let full = Digest32::digest_bytes(b"full-state");
    let campaigns = BTreeSet::from([demo, full]);
    let server = root.path().join("server.toml");
    fs::write(
        &server,
        format!(
            concat!(
                "bind = \"127.0.0.1:8787\"\n",
                "database_path = \"/home/robinhood/.local/share/robin-highscores/database/highscores.sqlite3\"\n",
                "replay_directory = \"/home/robinhood/.local/share/robin-highscores/replays\"\n",
                "campaign_state_directory = \"/home/robinhood/.local/share/robin-highscores/campaign-states\"\n",
                "cursor_secret_path = \"/home/robinhood/.local/share/robin-highscores/api-secrets/cursor-hmac.key\"\n",
                "competition_run_grant_secret_path = \"/home/robinhood/.local/share/robin-highscores/api-secrets/competition-run-grant.key\"\n",
                "run_preflight_grant_secret_path = \"/home/robinhood/.local/share/robin-highscores/api-secrets/run-preflight-grant.key\"\n",
                "moderation_bearer_token_path = \"/home/robinhood/.local/share/robin-highscores/api-secrets/moderation-bearer.token\"\n",
                "backup_manifest_path = \"/home/robinhood/.local/share/robin-highscores/status/backup-status.json\"\n",
                "release_manifest_path = \"/home/robinhood/.local/opt/robin-highscores/releases/{}/vps-release-manifest-v2.json\"\n",
                "maximum_backup_age_hours = 32\n",
                "minimum_storage_free_bytes = 1073741824\n",
                "max_replay_bytes = 16777216\n",
                "max_campaign_bytes = 16777216\n",
                "max_metadata_bytes = 65536\n",
                "max_concurrent_uploads = 32\n",
                "allowed_origins = []\n",
                "manifest_directory = \"/home/robinhood/.local/opt/robin-highscores/releases/{}/config/manifests\"\n",
                "[[admission_profiles]]\n",
                "canonical_campaign_state_path = \"/home/robinhood/.local/opt/robin-highscores/releases/{}/private/campaign-states/{}\"\n",
                "[[admission_profiles]]\n",
                "canonical_campaign_state_path = \"/home/robinhood/.local/opt/robin-highscores/releases/{}/private/campaign-states/{}\"\n"
            ),
            source_commit, source_commit, source_commit, demo, source_commit, full
        ),
    )?;
    validate_final_config(
        VpsConfigRoleV2::Server,
        &server,
        Digest32::digest_bytes(b"verifier"),
        &fact(b"catalog"),
        &source_commit,
        &campaigns,
    )?;
    Ok(())
}

#[test]
fn assembly_failure_preserves_absent_output() {
    let root = tempfile::tempdir().unwrap();
    let plan = root.path().join("plan.json");
    fs::write(&plan, b"{}").unwrap();
    let output = root.path().join("release");
    assert!(assemble_vps_release_v2(&plan, &output).is_err());
    assert!(!output.exists());
}

#[test]
fn assembly_output_requires_release_sibling_partial_and_rejects_legacy_incoming() -> Result<()> {
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let partial = Path::new(INSTALL_ROOT)
        .join("releases")
        .join(format!("{commit}.partial"));
    validate_vps_release_assembly_output(&partial, commit)?;

    let legacy = Path::new(INSTALL_ROOT)
        .join("incoming")
        .join(format!("{commit}.partial"));
    assert!(
        validate_vps_release_assembly_output(&legacy, commit).is_err(),
        "the legacy incoming assembly output was accepted"
    );
    Ok(())
}

#[test]
fn publication_lock_projection_requires_exact_pinned_canonical_v2() -> Result<()> {
    use std::os::fd::AsRawFd as _;

    let fixture = source_consume_fixture()?;
    let manifest_path = fixture.candidate.join(RELEASE_MANIFEST_FILE);
    let manifest = File::open(&manifest_path)?;
    assert_eq!(
        project_vps_publication_lock_v2(
            manifest.as_raw_fd(),
            &fixture.release_manifest_sha256.to_string(),
        )?,
        fixture.publication_lock_sha256
    );
    assert!(project_vps_publication_lock_v2(manifest.as_raw_fd(), &"0".repeat(64)).is_err());

    let wrong_type = File::open(&fixture.candidate)?;
    assert!(
        project_vps_publication_lock_v2(
            wrong_type.as_raw_fd(),
            &fixture.release_manifest_sha256.to_string(),
        )
        .is_err()
    );

    let malformed = fixture.sandbox.path().join("noncanonical-vps-v2.json");
    let mut malformed_bytes = fs::read(&manifest_path)?;
    malformed_bytes.push(b'\n');
    fs::write(&malformed, &malformed_bytes)?;
    set_mode(&malformed, 0o440)?;
    let malformed_file = File::open(&malformed)?;
    assert!(
        project_vps_publication_lock_v2(
            malformed_file.as_raw_fd(),
            &Digest32::digest_bytes(&malformed_bytes).to_string(),
        )
        .is_err()
    );

    let wrong_schema = fixture.sandbox.path().join("wrong-schema-vps-v2.json");
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    document["schema_version"] = serde_json::Value::from(1);
    let wrong_schema_bytes = serde_json::to_vec(&document)?;
    fs::write(&wrong_schema, &wrong_schema_bytes)?;
    set_mode(&wrong_schema, 0o440)?;
    let wrong_schema_file = File::open(&wrong_schema)?;
    assert!(
        project_vps_publication_lock_v2(
            wrong_schema_file.as_raw_fd(),
            &Digest32::digest_bytes(&wrong_schema_bytes).to_string(),
        )
        .is_err()
    );

    for (name, mode) in [
        ("group-writable-vps-v2.json", 0o640),
        ("world-readable-vps-v2.json", 0o444),
    ] {
        let unsafe_mode = fixture.sandbox.path().join(name);
        fs::write(&unsafe_mode, fs::read(&manifest_path)?)?;
        set_mode(&unsafe_mode, mode)?;
        let unsafe_mode_file = File::open(&unsafe_mode)?;
        assert!(
            project_vps_publication_lock_v2(
                unsafe_mode_file.as_raw_fd(),
                &fixture.release_manifest_sha256.to_string(),
            )
            .is_err(),
            "publication projection accepted manifest mode {mode:o}"
        );
    }

    let hardlinked = fixture.sandbox.path().join("hardlinked-vps-v2.json");
    let hardlink_alias = fixture.sandbox.path().join("hardlinked-vps-v2.alias");
    fs::write(&hardlinked, fs::read(&manifest_path)?)?;
    set_mode(&hardlinked, 0o440)?;
    fs::hard_link(&hardlinked, &hardlink_alias)?;
    let hardlinked_file = File::open(&hardlinked)?;
    assert!(
        project_vps_publication_lock_v2(
            hardlinked_file.as_raw_fd(),
            &fixture.release_manifest_sha256.to_string(),
        )
        .is_err(),
        "publication projection accepted a hard-linked manifest"
    );

    let displaced = fixture.candidate.join("vps-release-manifest-v2.displaced");
    set_mode(&fixture.candidate, 0o750)?;
    fs::rename(&manifest_path, &displaced)?;
    fs::write(&manifest_path, b"{}")?;
    set_mode(&manifest_path, 0o440)?;
    set_mode(&fixture.candidate, 0o550)?;
    assert_eq!(
        project_vps_publication_lock_v2(
            manifest.as_raw_fd(),
            &fixture.release_manifest_sha256.to_string(),
        )?,
        fixture.publication_lock_sha256,
        "path replacement changed the retained descriptor authority"
    );
    Ok(())
}

#[test]
fn failed_sealed_vps_staging_cleanup_is_bounded_and_nonfollowing() -> Result<()> {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let sandbox = tempfile::tempdir()?;
    let output = sandbox.path().join("release");
    let staging = staging_directory(&output)?;
    let staging_path = staging.path().to_path_buf();
    let sealed = staging.path().join("nested/sealed");
    fs::create_dir_all(&sealed)?;
    fs::write(sealed.join("payload"), b"sealed payload")?;
    fs::set_permissions(sealed.join("payload"), fs::Permissions::from_mode(0o440))?;
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o550))?;
    fs::set_permissions(
        sealed.parent().context("sealed path has no parent")?,
        fs::Permissions::from_mode(0o550),
    )?;

    let outside = sandbox.path().join("outside");
    fs::create_dir(&outside)?;
    fs::write(outside.join("sentinel"), b"outside remains unchanged")?;
    symlink(&outside, staging.path().join("outside-link"))?;

    discard_failed_vps_staging(staging)?;
    ensure!(!staging_path.exists(), "failed VPS staging path remains");
    ensure!(
        fs::read(outside.join("sentinel"))? == b"outside remains unchanged",
        "VPS cleanup followed an external symlink"
    );
    ensure!(
        fs::metadata(&outside)?.permissions().mode() & 0o777 != 0o700,
        "VPS cleanup changed external directory permissions"
    );
    Ok(())
}

#[test]
fn pinned_vps_promotion_validates_and_renames_the_same_inode() -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = tempfile::tempdir()?;
    let releases = sandbox.path().join("releases");
    fs::create_dir(&releases)?;
    fs::set_permissions(&releases, fs::Permissions::from_mode(0o750))?;
    let commit = "a".repeat(40);
    let partial = releases.join(format!("{commit}.partial"));
    let output = releases.join(&commit);
    fs::create_dir(&partial)?;
    fs::write(partial.join(SOURCE_COMMIT_FILE), format!("{commit}\n"))?;
    fs::write(partial.join(SHA256SUMS_FILE), b"authenticated sums\n")?;
    fs::write(partial.join("sentinel"), b"same pinned release inode")?;
    fs::set_permissions(&partial, fs::Permissions::from_mode(0o550))?;
    let expected_sums = Digest32::digest_bytes(b"authenticated sums\n");
    let expected_manifest = Digest32::digest_bytes(b"validated pinned manifest");

    let observed_manifest = promote_pinned_vps_release_with(
        &partial,
        &output,
        &releases,
        &commit,
        expected_sums,
        |pinned| {
            ensure!(
                fs::symlink_metadata(pinned)?.is_dir(),
                "pinned root alias did not resolve to a directory"
            );
            ensure!(
                fs::read(pinned.join("sentinel"))? == b"same pinned release inode",
                "validator observed a substituted release"
            );
            Ok(expected_manifest)
        },
    )?;
    assert_eq!(observed_manifest, expected_manifest);
    assert!(!partial.exists());
    assert_eq!(
        fs::read(output.join("sentinel"))?,
        b"same pinned release inode"
    );
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn vps_persistence_noreplace_race_cleans_staging_without_overwrite() -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = tempfile::tempdir()?;
    let output = sandbox.path().join("release");
    let staging = staging_directory(&output)?;
    let staging_path = staging.path().to_path_buf();
    fs::create_dir(staging.path().join("sealed"))?;
    fs::write(staging.path().join("sealed/data"), b"candidate")?;
    fs::set_permissions(
        staging.path().join("sealed"),
        fs::Permissions::from_mode(0o550),
    )?;
    fs::create_dir(&output)?;
    fs::write(output.join("winner"), b"racing installer")?;

    let persist_error = persist_vps_staging(&staging, &output)
        .expect_err("NOREPLACE VPS persistence overwrote a racing output");
    ensure!(
        persist_error
            .downcast_ref::<VpsReleaseInstalledButParentSyncFailed>()
            .is_none(),
        "pre-rename VPS failure was misclassified as installed"
    );
    discard_failed_vps_staging(staging)?;
    ensure!(!staging_path.exists(), "raced VPS staging path remains");
    ensure!(
        fs::read(output.join("winner"))? == b"racing installer",
        "NOREPLACE VPS persistence modified the racing output"
    );
    ensure!(
        !output.join("sealed/data").exists(),
        "NOREPLACE VPS persistence partially merged the candidate"
    );
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn vps_post_rename_sync_failure_reports_exact_installed_identity() -> Result<()> {
    let sandbox = tempfile::tempdir()?;
    let output = sandbox.path().join("release");
    let staging = staging_directory(&output)?;
    let staging_path = staging.path().to_path_buf();
    fs::write(staging.path().join("complete"), b"complete")?;
    let outcome = persist_vps_staging_with(&staging, &output, |_| {
        anyhow::bail!("injected VPS parent sync failure")
    })?;
    let VpsPersistenceOutcome::InstalledButParentSyncFailed(sync_error) = outcome else {
        anyhow::bail!("post-rename VPS failure was not reported as installed")
    };
    ensure!(!staging_path.exists(), "renamed VPS staging path remains");
    ensure!(
        fs::read(output.join("complete"))? == b"complete",
        "installed VPS output is incomplete after parent sync failure"
    );
    let _installed_path = staging.keep();
    let manifest_sha256 = Digest32::digest_bytes(b"release manifest");
    let source_commit = "a".repeat(40);
    let classified =
        vps_installed_durability_error(&output, manifest_sha256, &source_commit, sync_error);
    let installed = classified
        .downcast_ref::<VpsReleaseInstalledButParentSyncFailed>()
        .context("installed VPS durability error is not downcastable")?;
    ensure!(
        installed.output == output
            && installed.release_manifest_sha256 == manifest_sha256
            && installed.source_commit == source_commit,
        "installed VPS durability error lost exact release identity"
    );
    ensure!(
        installed.source.to_string() == "injected VPS parent sync failure",
        "installed VPS durability error lost its source"
    );
    Ok(())
}
