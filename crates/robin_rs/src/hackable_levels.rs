//! Discovery of hackable JSON levels shipped in overlay datadirs.
//!
//! A hackable level is a `Data/Levels/<mission>.level.json` descriptor (see
//! `robin_engine::level_data::HackableLevelDescriptor`) that expands into
//! normal level structs at load time. Discovered levels get a main-menu
//! entry and can be launched directly with `--mission <name>`.

use std::path::Path;

/// One launchable hackable level discovered in an overlay datadir.
#[derive(Debug, Clone)]
pub struct HackableLevel {
    /// Mission filename, i.e. the `<mission>` in
    /// `Data/Levels/<mission>.level.json`, usable with `--mission`.
    pub mission: String,
    /// Menu display name from the descriptor's `title`, falling back to
    /// the mission filename.
    pub title: String,
}

const DESCRIPTOR_SUFFIX: &str = ".level.json";

/// Enumerate hackable levels across all registered overlay datadirs.
///
/// Only overlays are scanned: original game datadirs never contain
/// descriptors, and `SbFile` offers no directory listing, so this walks the
/// overlay roots with `std::fs` directly. Descriptors that fail to parse are
/// skipped with a warning rather than hiding the whole menu section. The
/// first overlay shipping a mission name wins, matching `SbFile` lookup
/// order; the result is sorted by title for a stable menu layout.
pub fn discover_hackable_levels() -> Vec<HackableLevel> {
    let mut levels: Vec<HackableLevel> = Vec::new();
    for root in robin_engine::sbfile::SbFile::overlay_paths() {
        let dir = Path::new(&root).join("Data").join("Levels");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            let Some(mission) = file_name.strip_suffix(DESCRIPTOR_SUFFIX) else {
                continue;
            };
            if mission.is_empty()
                || levels
                    .iter()
                    .any(|level| level.mission.eq_ignore_ascii_case(mission))
            {
                continue;
            }
            let path = entry.path();
            let descriptor = std::fs::read(&path)
                .map_err(|error| error.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<robin_engine::level_data::HackableLevelDescriptor>(
                        &bytes,
                    )
                    .map_err(|error| error.to_string())
                });
            let descriptor = match descriptor {
                Ok(descriptor) => descriptor,
                Err(error) => {
                    tracing::warn!(
                        "Skipping unreadable hackable level descriptor {}: {error}",
                        path.display()
                    );
                    continue;
                }
            };
            levels.push(HackableLevel {
                mission: mission.to_owned(),
                title: descriptor.title.unwrap_or_else(|| mission.to_owned()),
            });
        }
    }
    levels.sort_by(|a, b| {
        a.title
            .cmp(&b.title)
            .then_with(|| a.mission.cmp(&b.mission))
    });
    levels
}
