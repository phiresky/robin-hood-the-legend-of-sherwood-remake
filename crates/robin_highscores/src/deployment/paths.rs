//! Fixed production deployment identities; these are policy, not runtime authority.

// TODO: STATE_ROOT is only referenced by robin_manifest_tool::vps_release_v2,
// which the parallel lane deletes; drop it once that merges.
pub const STATE_ROOT: &str = "/home/robinhood/.local/share/robin-highscores";
pub const DEMO_RAW_CONTENT_ROOT: &str =
    "/home/robinhood/.local/share/robin-highscores/raw-content/demo";
pub const FULL_RAW_CONTENT_ROOT: &str =
    "/home/robinhood/.local/share/robin-highscores/raw-content/full";
