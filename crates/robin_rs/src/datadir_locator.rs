//! Locate a game installation at startup when no datadir was given.
//!
//! Used by `setup_data_dir` when `ROBINHOOD_DATA_DIR` is unset and neither
//! the working directory nor the executable directory contains the game
//! data: first probe the usual install locations of the CD, GOG, and Steam
//! releases, then fall back to asking the player through the native OS
//! folder picker.
//!
//! A directory counts as a game installation if and only if it contains
//! `Data/robinhood.bks` in any capitalization — the sprite-bank index that
//! every release ships and nothing else plausibly provides.

use std::path::{Path, PathBuf};

/// Where to buy the game; shown in the picker dialog.
pub const GOG_STORE_URL: &str = "https://www.gog.com/game/robin_hood_the_legend_of_sherwood";

/// Marker file identifying a correct datadir, looked up case-insensitively
/// inside the installation's `Data/` folder.
const MARKER_FILE: &str = "robinhood.bks";

/// Install folder names used by the known Windows distributions. These are
/// joined onto every searched root, so each spelling only needs to be
/// listed once.
const INSTALL_FOLDER_NAMES: &[&str] = &[
    // GOG offline installer / Galaxy, and the localized CD releases.
    "Robin Hood - The Legend of Sherwood",
    // Steam `installdir`.
    "Robin Hood The Legend of Sherwood",
    // Original CD installer shorthand.
    "Robin Hood",
];

/// Case-insensitive single-component lookup: the entry of `dir` whose name
/// matches `name` ignoring ASCII case.
fn entry_case_insensitive(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.find_map(|entry| {
        let entry = entry.ok()?;
        entry
            .file_name()
            .to_str()?
            .eq_ignore_ascii_case(name)
            .then(|| entry.path())
    })
}

/// True when `dir` is a game installation root containing
/// `Data/robinhood.bks` (any capitalization of either component), or a
/// pre-converted shipping bundle (`Data/datadir.bin`), which replaces the
/// loose files and therefore has no `.bks`.
pub fn is_valid_install_dir(dir: &Path) -> bool {
    entry_case_insensitive(dir, "Data")
        .map(|data| {
            entry_case_insensitive(&data, MARKER_FILE).is_some()
                || entry_case_insensitive(&data, "datadir.bin").is_some()
        })
        .unwrap_or(false)
}

/// Roots under which [`INSTALL_FOLDER_NAMES`] are searched, plus the store
/// layouts (`GOG Games/`, Galaxy's and Steam's library folders).
fn search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut push = |root: PathBuf| {
        if root.is_dir() && !roots.contains(&root) {
            roots.push(root);
        }
    };

    #[cfg(target_os = "windows")]
    {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(dir) = std::env::var_os(var).map(PathBuf::from) {
                push(dir.join("GOG Galaxy").join("Games"));
                // Older GOG offline installers defaulted here.
                push(dir.join("GOG.com"));
                push(dir.join("Steam").join("steamapps").join("common"));
                push(dir);
            }
        }
        let system_drive = std::env::var_os("SystemDrive")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:"));
        // `C:` alone is cwd-relative on Windows; re-anchor at the root.
        let drive_root = PathBuf::from(format!("{}\\", system_drive.display()));
        push(drive_root.join("GOG Games"));
        push(drive_root.join("Games"));
    }

    #[cfg(not(target_os = "windows"))]
    {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return roots;
        };
        // GOG via MojoSetup / Heroic / Lutris under Wine.
        push(home.join("GOG Games"));
        push(home.join("Games/Heroic"));
        push(home.join("Games"));
        push(home.join(".wine/drive_c/GOG Games"));
        push(home.join(".wine/drive_c/Program Files (x86)/GOG.com"));
        push(home.join(".wine/drive_c/Program Files (x86)/GOG Galaxy/Games"));
        // Steam library folders (native client layouts).
        push(home.join(".local/share/Steam/steamapps/common"));
        push(home.join(".steam/steam/steamapps/common"));
        #[cfg(target_os = "macos")]
        push(home.join("Library/Application Support/Steam/steamapps/common"));
    }

    roots
}

/// Probe the well-known install locations of the CD, GOG, and Steam
/// releases and return the first valid installation.
pub fn find_installed_datadir() -> Option<PathBuf> {
    for root in search_roots() {
        for name in INSTALL_FOLDER_NAMES {
            let Some(candidate) = entry_case_insensitive(&root, name) else {
                continue;
            };
            if is_valid_install_dir(&candidate) {
                tracing::info!("Found game installation: {}", candidate.display());
                return Some(candidate);
            }
        }
    }
    None
}

/// Accept a picked folder as either the installation root or its `Data`
/// subfolder (players often select `Data` itself), returning the root.
fn normalize_selection(path: &Path) -> Option<PathBuf> {
    if is_valid_install_dir(path) {
        return Some(path.to_owned());
    }
    if entry_case_insensitive(path, MARKER_FILE).is_some() {
        return path.parent().map(Path::to_owned);
    }
    None
}

/// Whether a native dialog can appear at all. Prevents headless runs
/// (CI, batch tools without a datadir) from hanging on an invisible
/// prompt.
fn display_available() -> bool {
    if cfg!(target_os = "linux") {
        std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
    } else {
        true
    }
}

/// Ask the player for the installation folder with the native OS picker.
/// Loops until a valid folder is chosen or the player cancels.
pub fn prompt_for_datadir() -> Option<PathBuf> {
    if !display_available() {
        tracing::warn!("No display available; skipping the datadir picker dialog");
        return None;
    }
    loop {
        let choice = pollster::block_on(
            rfd::AsyncMessageDialog::new()
                .set_level(rfd::MessageLevel::Info)
                .set_title("Robin Hood — game data not found")
                .set_description(format!(
                    "This is an open-source engine for Robin Hood: The Legend of Sherwood; \
                     it needs the original game's data files, and no installation was found.\n\n\
                     Click OK to select the folder where the game is installed \
                     (the one containing Data/robinhood.bks).\n\n\
                     I recommend to buy it on GOG if you do not have it:\n{GOG_STORE_URL}\n\
                     It is also available on Steam, but the distributers there do not care \
                     about it — it breaks on modern Windows without tweaks."
                ))
                .set_buttons(rfd::MessageButtons::OkCancel)
                .show(),
        );
        if choice != rfd::MessageDialogResult::Ok {
            return None;
        }
        let Some(folder) = pollster::block_on(
            rfd::AsyncFileDialog::new()
                .set_title("Select the Robin Hood installation folder")
                .pick_folder(),
        ) else {
            return None;
        };
        let picked = folder.path().to_path_buf();
        if let Some(install_dir) = normalize_selection(&picked) {
            tracing::info!(
                "Player selected game installation: {}",
                install_dir.display()
            );
            return Some(install_dir);
        }
        pollster::block_on(
            rfd::AsyncMessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Not a Robin Hood installation")
                .set_description(format!(
                    "{} does not contain Data/{MARKER_FILE}.\n\
                     Please select the game's installation folder.",
                    picked.display()
                ))
                .set_buttons(rfd::MessageButtons::Ok)
                .show(),
        );
    }
}

/// Full fallback chain for a missing datadir: known install locations
/// first, then the native folder picker.
pub fn locate_or_prompt() -> Option<PathBuf> {
    find_installed_datadir().or_else(prompt_for_datadir)
}
