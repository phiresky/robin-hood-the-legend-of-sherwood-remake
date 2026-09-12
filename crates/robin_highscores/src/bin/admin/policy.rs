//! Fixed deployment paths and backup format policy; no runtime authority.

pub(super) const BACKUP_MANIFEST_SCHEMA_VERSION: u32 = 4;

pub(super) use robin_highscores::deployment::paths::{
    DEFAULT_BACKUP_AUTHORITY_KEY, DEFAULT_BACKUP_ROOT, DEFAULT_BACKUP_STATUS,
    INSTALLED_RELEASE_ROOT, SYSTEMD_USER_ROOT, VPS_ACTIVATION_ROOT,
};

pub(super) const RELEASE_AUTHORITY_STORE: &str = ".release-authorities-v2";

pub(super) const SYSTEMD_UNIT_FILES: [&str; 5] = [
    "robin-highscores.target",
    "robin-highscores-api.service",
    "robin-highscores-worker.service",
    "robin-highscores-backup.service",
    "robin-highscores-backup.timer",
];
