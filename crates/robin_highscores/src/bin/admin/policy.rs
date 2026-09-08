//! Fixed deployment paths and backup format policy; no runtime authority.

pub(super) const BACKUP_MANIFEST_SCHEMA_VERSION: u32 = 4;

pub(super) const DEFAULT_BACKUP_ROOT: &str =
    "/home/robinhood/.local/share/robin-highscores/backups";

pub(super) const DEFAULT_BACKUP_STATUS: &str =
    "/home/robinhood/.local/share/robin-highscores/status/backup-status.json";

pub(super) const DEFAULT_BACKUP_AUTHORITY_KEY: &str =
    "/home/robinhood/.local/share/robin-highscores/api-secrets/backup-authority-hmac.key";

pub(super) const VPS_ACTIVATION_ROOT: &str = "/home/robinhood/.local/opt/robin-highscores";

pub(super) const INSTALLED_RELEASE_ROOT: &str =
    "/home/robinhood/.local/opt/robin-highscores/releases";

pub(super) const RELEASE_AUTHORITY_STORE: &str = ".release-authorities-v2";

pub(super) const SYSTEMD_USER_ROOT: &str = "/home/robinhood/.config/systemd/user";

pub(super) const SYSTEMD_UNIT_FILES: [&str; 5] = [
    "robin-highscores.target",
    "robin-highscores-api.service",
    "robin-highscores-worker.service",
    "robin-highscores-backup.service",
    "robin-highscores-backup.timer",
];
