use super::*;
use robin_run_protocol::canonical_json_bytes;
use std::path::Path;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
struct BackupAuthorityKeyHarness {
    _directory: tempfile::TempDir,
    opt_root: PathBuf,
    key_path: PathBuf,
    activation_lock: std::fs::File,
}

#[cfg(target_os = "linux")]
impl BackupAuthorityKeyHarness {
    fn new() -> Self {
        use fs2::FileExt as _;
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

        let directory = tempfile::tempdir().unwrap();
        let opt_root = directory.path().join("opt/robin-highscores");
        let secret_parent = directory.path().join("state/api-secrets");
        std::fs::create_dir_all(&opt_root).unwrap();
        std::fs::create_dir_all(&secret_parent).unwrap();
        std::fs::set_permissions(&opt_root, std::fs::Permissions::from_mode(0o750)).unwrap();
        std::fs::set_permissions(&secret_parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        let lock_path = opt_root.join("activation.lock");
        let activation_lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&lock_path)
            .unwrap();
        std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        activation_lock.sync_all().unwrap();
        std::fs::File::open(&opt_root).unwrap().sync_all().unwrap();
        activation_lock.lock_exclusive().unwrap();
        Self {
            _directory: directory,
            opt_root,
            key_path: secret_parent.join("backup-authority-hmac.key"),
            activation_lock,
        }
    }

    fn activation_lock_fd(&self) -> u32 {
        use std::os::fd::AsRawFd as _;
        u32::try_from(self.activation_lock.as_raw_fd()).unwrap()
    }

    fn initialize(&self, source_commit: &str) -> anyhow::Result<()> {
        initialize_backup_authority_key_v2_at(
            &self.opt_root,
            &self.key_path,
            rustix::process::geteuid().as_raw(),
            source_commit,
            self.activation_lock_fd(),
            |_| Ok(()),
        )
    }

    fn interrupt_initialize(
        &self,
        source_commit: &str,
        boundary: BackupAuthorityKeyBoundary,
    ) -> anyhow::Result<()> {
        let mut interrupted = false;
        initialize_backup_authority_key_v2_at(
            &self.opt_root,
            &self.key_path,
            rustix::process::geteuid().as_raw(),
            source_commit,
            self.activation_lock_fd(),
            |observed| {
                if observed == boundary && !interrupted {
                    interrupted = true;
                    anyhow::bail!("injected interruption at {observed:?}");
                }
                Ok(())
            },
        )
    }

    fn complete<V>(&self, source_commit: &str, validate: V) -> anyhow::Result<()>
    where
        V: FnOnce() -> anyhow::Result<()>,
    {
        complete_backup_authority_key_v2_at(
            &self.opt_root,
            &self.key_path,
            rustix::process::geteuid().as_raw(),
            source_commit,
            self.activation_lock_fd(),
            validate,
            |_| Ok(()),
        )
    }

    fn interrupt_complete<V>(
        &self,
        source_commit: &str,
        validate: V,
        boundary: BackupAuthorityKeyBoundary,
    ) -> anyhow::Result<()>
    where
        V: FnOnce() -> anyhow::Result<()>,
    {
        let mut interrupted = false;
        complete_backup_authority_key_v2_at(
            &self.opt_root,
            &self.key_path,
            rustix::process::geteuid().as_raw(),
            source_commit,
            self.activation_lock_fd(),
            validate,
            |observed| {
                if observed == boundary && !interrupted {
                    interrupted = true;
                    anyhow::bail!("injected interruption at {observed:?}");
                }
                Ok(())
            },
        )
    }

    fn intent_path(&self) -> PathBuf {
        self.key_path
            .parent()
            .unwrap()
            .join(BACKUP_AUTHORITY_INTENT_NAME)
    }

    fn temporary_intent_path(&self) -> PathBuf {
        self.key_path
            .parent()
            .unwrap()
            .join(BACKUP_AUTHORITY_INTENT_TEMP_NAME)
    }
}

#[cfg(target_os = "linux")]
const TEST_SOURCE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[cfg(target_os = "linux")]
fn test_anonymous_backup_authority_key(parent: &PinnedSecretParent) -> std::fs::File {
    use rustix::fs::{Mode, OFlags, fchmod, openat};
    use std::io::Write as _;
    use std::os::fd::AsFd as _;

    let anonymous = openat(
        parent.fd.as_fd(),
        ".",
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::TMPFILE,
        Mode::from_raw_mode(0o400),
    )
    .unwrap();
    fchmod(&anonymous, Mode::from_raw_mode(0o400)).unwrap();
    let mut anonymous = std::fs::File::from(anonymous);
    anonymous.write_all(&[0x5a; 32]).unwrap();
    anonymous.sync_all().unwrap();
    anonymous
}

#[cfg(target_os = "linux")]
#[test]
fn backup_authority_key_publication_falls_back_from_empty_path_enoent() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let harness = BackupAuthorityKeyHarness::new();
    let parent = pin_secret_parent(&harness.key_path, rustix::process::geteuid().as_raw()).unwrap();
    let anonymous = test_anonymous_backup_authority_key(&parent);
    let retained = anonymous.metadata().unwrap();
    publish_anonymous_backup_authority_key_with(
        &anonymous,
        &parent,
        "backup-authority-hmac.key",
        Path::new("/proc/self/fd"),
        || Err(rustix::io::Errno::NOENT),
    )
    .unwrap();
    let published = std::fs::symlink_metadata(&harness.key_path).unwrap();
    assert_eq!(published.dev(), retained.dev());
    assert_eq!(published.ino(), retained.ino());
    assert_eq!(published.uid(), retained.uid());
    assert_eq!(published.nlink(), 1);
    assert_eq!(published.permissions().mode() & 0o777, 0o400);
    assert_eq!(published.len(), 32);
}

#[cfg(target_os = "linux")]
#[test]
fn backup_authority_key_proc_fallback_rejects_missing_and_substituted_descriptors() {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};

    let missing = BackupAuthorityKeyHarness::new();
    let missing_parent =
        pin_secret_parent(&missing.key_path, rustix::process::geteuid().as_raw()).unwrap();
    let missing_anonymous = test_anonymous_backup_authority_key(&missing_parent);
    let missing_proc = tempfile::tempdir().unwrap();
    assert!(
        publish_anonymous_backup_authority_key_with(
            &missing_anonymous,
            &missing_parent,
            "backup-authority-hmac.key",
            missing_proc.path(),
            || Err(rustix::io::Errno::NOENT),
        )
        .is_err()
    );
    assert!(!missing.key_path.exists());

    let substituted = BackupAuthorityKeyHarness::new();
    let substituted_parent =
        pin_secret_parent(&substituted.key_path, rustix::process::geteuid().as_raw()).unwrap();
    let substituted_anonymous = test_anonymous_backup_authority_key(&substituted_parent);
    let fake_proc = tempfile::tempdir().unwrap();
    let other = fake_proc.path().join("other");
    std::fs::write(&other, [0x33; 32]).unwrap();
    std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o400)).unwrap();
    let before = std::fs::metadata(&other).unwrap();
    symlink(
        &other,
        fake_proc
            .path()
            .join(substituted_anonymous.as_raw_fd().to_string()),
    )
    .unwrap();
    assert!(
        publish_anonymous_backup_authority_key_with(
            &substituted_anonymous,
            &substituted_parent,
            "backup-authority-hmac.key",
            fake_proc.path(),
            || Err(rustix::io::Errno::NOENT),
        )
        .is_err()
    );
    assert!(!substituted.key_path.exists());
    assert_eq!(std::fs::metadata(&other).unwrap().nlink(), before.nlink());
}

#[cfg(target_os = "linux")]
#[test]
fn backup_authority_key_proc_fallback_never_replaces_an_existing_destination() {
    use std::os::unix::fs::PermissionsExt as _;

    let harness = BackupAuthorityKeyHarness::new();
    std::fs::write(&harness.key_path, b"existing authority").unwrap();
    std::fs::set_permissions(&harness.key_path, std::fs::Permissions::from_mode(0o400)).unwrap();
    let parent = pin_secret_parent(&harness.key_path, rustix::process::geteuid().as_raw()).unwrap();
    let anonymous = test_anonymous_backup_authority_key(&parent);
    assert!(
        publish_anonymous_backup_authority_key_with(
            &anonymous,
            &parent,
            "backup-authority-hmac.key",
            Path::new("/proc/self/fd"),
            || Err(rustix::io::Errno::NOENT),
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(&harness.key_path).unwrap(),
        b"existing authority"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn backup_authority_key_proc_fallback_does_not_mask_policy_errors() {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::symlink;

    for direct_error in [rustix::io::Errno::PERM, rustix::io::Errno::ACCESS] {
        let harness = BackupAuthorityKeyHarness::new();
        let parent =
            pin_secret_parent(&harness.key_path, rustix::process::geteuid().as_raw()).unwrap();
        let anonymous = test_anonymous_backup_authority_key(&parent);
        let fake_proc = tempfile::tempdir().unwrap();
        symlink(
            format!("/proc/self/fd/{}", anonymous.as_raw_fd()),
            fake_proc.path().join(anonymous.as_raw_fd().to_string()),
        )
        .unwrap();

        let error = publish_anonymous_backup_authority_key_with(
            &anonymous,
            &parent,
            "backup-authority-hmac.key",
            fake_proc.path(),
            || Err(direct_error),
        )
        .unwrap_err();
        assert!(error.to_string().contains("publish anonymous"));
        assert!(!harness.key_path.exists());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn backup_authority_key_v2_resumes_every_initialization_publication_boundary() {
    for boundary in [
        BackupAuthorityKeyBoundary::AnonymousKeySynced,
        BackupAuthorityKeyBoundary::IntentTemporaryCreated,
        BackupAuthorityKeyBoundary::IntentTemporaryWritten,
        BackupAuthorityKeyBoundary::IntentTemporarySynced,
        BackupAuthorityKeyBoundary::IntentPublished,
        BackupAuthorityKeyBoundary::KeyLinked,
        BackupAuthorityKeyBoundary::KeyDirectorySynced,
        BackupAuthorityKeyBoundary::KeyValidated,
    ] {
        let harness = BackupAuthorityKeyHarness::new();
        assert!(
            harness
                .interrupt_initialize(TEST_SOURCE_COMMIT, boundary)
                .is_err(),
            "boundary {boundary:?} was not reached"
        );
        harness.initialize(TEST_SOURCE_COMMIT).unwrap();
        let key = std::fs::read(&harness.key_path).unwrap();
        assert_eq!(key.len(), 32);
        assert!(key.iter().any(|byte| *byte != 0));
        assert!(harness.intent_path().is_file());
        assert!(!harness.temporary_intent_path().exists());
        harness.initialize(TEST_SOURCE_COMMIT).unwrap();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn backup_authority_key_v2_resumes_both_recovery_mutation_boundaries() {
    let temporary = BackupAuthorityKeyHarness::new();
    assert!(
        temporary
            .interrupt_initialize(
                TEST_SOURCE_COMMIT,
                BackupAuthorityKeyBoundary::IntentTemporarySynced,
            )
            .is_err()
    );
    assert!(
        temporary
            .interrupt_initialize(
                TEST_SOURCE_COMMIT,
                BackupAuthorityKeyBoundary::RecoveryTemporaryRemoved,
            )
            .is_err()
    );
    temporary.initialize(TEST_SOURCE_COMMIT).unwrap();

    let intent = BackupAuthorityKeyHarness::new();
    assert!(
        intent
            .interrupt_initialize(
                TEST_SOURCE_COMMIT,
                BackupAuthorityKeyBoundary::IntentPublished,
            )
            .is_err()
    );
    assert!(
        intent
            .interrupt_initialize(
                TEST_SOURCE_COMMIT,
                BackupAuthorityKeyBoundary::RecoveryIntentRemoved,
            )
            .is_err()
    );
    intent.initialize(TEST_SOURCE_COMMIT).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn backup_authority_key_v2_completion_requires_outer_authority_and_is_resumable() {
    use std::cell::Cell;

    for boundary in [
        BackupAuthorityKeyBoundary::OuterAuthorityValidated,
        BackupAuthorityKeyBoundary::CompletionIntentRemoved,
        BackupAuthorityKeyBoundary::CompletionDirectorySynced,
    ] {
        let harness = BackupAuthorityKeyHarness::new();
        harness.initialize(TEST_SOURCE_COMMIT).unwrap();
        assert!(harness.intent_path().is_file());
        assert!(
            harness
                .interrupt_complete(TEST_SOURCE_COMMIT, || Ok(()), boundary)
                .is_err(),
            "completion boundary {boundary:?} was not reached"
        );
        let calls = Cell::new(0);
        harness
            .complete(TEST_SOURCE_COMMIT, || {
                calls.set(calls.get() + 1);
                Ok(())
            })
            .unwrap();
        assert_eq!(calls.get(), 1);
        assert!(!harness.intent_path().exists());
        assert_eq!(std::fs::read(&harness.key_path).unwrap().len(), 32);
        harness.complete(TEST_SOURCE_COMMIT, || Ok(())).unwrap();
    }

    let rejected = BackupAuthorityKeyHarness::new();
    rejected.initialize(TEST_SOURCE_COMMIT).unwrap();
    assert!(
        rejected
            .complete(TEST_SOURCE_COMMIT, || anyhow::bail!(
                "outer authority absent"
            ))
            .is_err()
    );
    assert!(rejected.intent_path().is_file());
}

#[cfg(target_os = "linux")]
#[test]
fn backup_authority_key_v2_requires_the_exact_held_lock_open_file_description() {
    use fs2::FileExt as _;
    use std::os::fd::AsRawFd as _;

    let harness = BackupAuthorityKeyHarness::new();
    let wrong = BackupAuthorityKeyHarness::new();
    assert!(
        initialize_backup_authority_key_v2_at(
            &harness.opt_root,
            &harness.key_path,
            rustix::process::geteuid().as_raw(),
            TEST_SOURCE_COMMIT,
            u32::try_from(wrong.activation_lock.as_raw_fd()).unwrap(),
            |_| Ok(()),
        )
        .is_err(),
        "a lock descriptor for another canonical root was accepted"
    );

    harness.activation_lock.unlock().unwrap();
    assert!(harness.initialize(TEST_SOURCE_COMMIT).is_err());
    harness.activation_lock.lock_exclusive().unwrap();

    let reopened = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(harness.opt_root.join("activation.lock"))
        .unwrap();
    assert!(
        initialize_backup_authority_key_v2_at(
            &harness.opt_root,
            &harness.key_path,
            rustix::process::geteuid().as_raw(),
            TEST_SOURCE_COMMIT,
            u32::try_from(reopened.as_raw_fd()).unwrap(),
            |_| Ok(()),
        )
        .is_err(),
        "a reopened canonical lock inode with a distinct OFD was accepted"
    );
    harness.initialize(TEST_SOURCE_COMMIT).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn backup_authority_key_v2_rejects_stale_and_unjournaled_authority() {
    let stale = BackupAuthorityKeyHarness::new();
    assert!(
        stale
            .interrupt_initialize(
                TEST_SOURCE_COMMIT,
                BackupAuthorityKeyBoundary::IntentPublished,
            )
            .is_err()
    );
    assert!(
        stale
            .initialize("89abcdef0123456789abcdef0123456789abcdef")
            .is_err(),
        "a stale intent from another source transaction was discarded"
    );
    assert!(stale.intent_path().is_file());

    let unjournaled = BackupAuthorityKeyHarness::new();
    unjournaled.initialize(TEST_SOURCE_COMMIT).unwrap();
    std::fs::remove_file(unjournaled.intent_path()).unwrap();
    assert!(
        unjournaled.initialize(TEST_SOURCE_COMMIT).is_err(),
        "initialization silently adopted a final key without intent"
    );
    assert!(
        unjournaled
            .complete(TEST_SOURCE_COMMIT, || anyhow::bail!(
                "runtime authority absent"
            ))
            .is_err()
    );
    unjournaled.complete(TEST_SOURCE_COMMIT, || Ok(())).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn backup_authority_key_v2_rejects_key_symlink_hardlink_mode_and_content_substitution() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    for mutation in ["symlink", "hardlink", "mode", "content"] {
        let harness = BackupAuthorityKeyHarness::new();
        harness.initialize(TEST_SOURCE_COMMIT).unwrap();
        match mutation {
            "symlink" => {
                let displaced = harness.key_path.with_extension("displaced");
                std::fs::rename(&harness.key_path, &displaced).unwrap();
                symlink(&displaced, &harness.key_path).unwrap();
            }
            "hardlink" => {
                std::fs::hard_link(&harness.key_path, harness.key_path.with_extension("alias"))
                    .unwrap();
            }
            "mode" => {
                std::fs::set_permissions(&harness.key_path, std::fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
            "content" => {
                std::fs::set_permissions(&harness.key_path, std::fs::Permissions::from_mode(0o600))
                    .unwrap();
                std::fs::write(&harness.key_path, [0x5a; 32]).unwrap();
                std::fs::set_permissions(&harness.key_path, std::fs::Permissions::from_mode(0o400))
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            harness.initialize(TEST_SOURCE_COMMIT).is_err(),
            "{mutation} key substitution was accepted"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn backup_authority_key_v2_rejects_intent_and_owner_substitution() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    for mutation in ["symlink", "hardlink", "mode"] {
        let harness = BackupAuthorityKeyHarness::new();
        harness.initialize(TEST_SOURCE_COMMIT).unwrap();
        match mutation {
            "symlink" => {
                let displaced = harness.intent_path().with_extension("displaced");
                std::fs::rename(harness.intent_path(), &displaced).unwrap();
                symlink(&displaced, harness.intent_path()).unwrap();
            }
            "hardlink" => {
                std::fs::hard_link(
                    harness.intent_path(),
                    harness.intent_path().with_extension("alias"),
                )
                .unwrap();
            }
            "mode" => {
                std::fs::set_permissions(
                    harness.intent_path(),
                    std::fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            harness.initialize(TEST_SOURCE_COMMIT).is_err(),
            "{mutation} intent substitution was accepted"
        );
    }

    let intent_content = BackupAuthorityKeyHarness::new();
    intent_content.initialize(TEST_SOURCE_COMMIT).unwrap();
    std::fs::set_permissions(
        intent_content.intent_path(),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let mut document: BackupAuthorityKeyIntentV1 =
        serde_json::from_slice(&std::fs::read(intent_content.intent_path()).unwrap()).unwrap();
    document.key_inode = document.key_inode.wrapping_add(1);
    std::fs::write(
        intent_content.intent_path(),
        canonical_json_bytes(&document).unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(
        intent_content.intent_path(),
        std::fs::Permissions::from_mode(0o400),
    )
    .unwrap();
    assert!(intent_content.initialize(TEST_SOURCE_COMMIT).is_err());

    let wrong_owner = BackupAuthorityKeyHarness::new();
    assert!(
        initialize_backup_authority_key_v2_at(
            &wrong_owner.opt_root,
            &wrong_owner.key_path,
            rustix::process::geteuid().as_raw().wrapping_add(1),
            TEST_SOURCE_COMMIT,
            wrong_owner.activation_lock_fd(),
            |_| Ok(()),
        )
        .is_err(),
        "a secret parent owned by a different expected identity was accepted"
    );
}
