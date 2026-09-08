use super::*;

#[cfg(unix)]
#[test]
fn operation_lock_rejects_symlink_hardlink_and_wrong_mode() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    drop(acquire_backup_operation_lock(root.path()).unwrap());
    let lock = root.path().join(".backup-operation.lock");
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert!(acquire_backup_operation_lock(root.path()).is_err());
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::hard_link(&lock, root.path().join("lock-alias")).unwrap();
    assert!(acquire_backup_operation_lock(root.path()).is_err());
    std::fs::remove_file(root.path().join("lock-alias")).unwrap();
    std::fs::remove_file(&lock).unwrap();
    let outside = root.path().join("outside-lock");
    std::fs::write(&outside, b"").unwrap();
    symlink(&outside, &lock).unwrap();
    assert!(acquire_backup_operation_lock(root.path()).is_err());
}
