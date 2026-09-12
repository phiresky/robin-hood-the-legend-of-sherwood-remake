//! Fixed production deployment identities; these are policy, not runtime authority.

pub const STATE_ROOT: &str = "/home/robinhood/.local/share/robin-highscores";
pub const VPS_ACTIVATION_ROOT: &str = "/home/robinhood/.local/opt/robin-highscores";
pub const INSTALLED_RELEASE_ROOT: &str = "/home/robinhood/.local/opt/robin-highscores/releases";
pub const DEFAULT_BACKUP_ROOT: &str = "/home/robinhood/.local/share/robin-highscores/backups";
pub const DEFAULT_BACKUP_STATUS: &str =
    "/home/robinhood/.local/share/robin-highscores/status/backup-status.json";
pub const DEFAULT_BACKUP_AUTHORITY_KEY: &str =
    "/home/robinhood/.local/share/robin-highscores/api-secrets/backup-authority-hmac.key";
pub const SYSTEMD_USER_ROOT: &str = "/home/robinhood/.config/systemd/user";
pub const DEMO_RAW_CONTENT_ROOT: &str =
    "/home/robinhood/.local/share/robin-highscores/raw-content/demo";
pub const FULL_RAW_CONTENT_ROOT: &str =
    "/home/robinhood/.local/share/robin-highscores/raw-content/full";
